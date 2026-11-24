
use crate::{
    hook::Hook,
    hook_errors,
    source_state::{self as state, Changed, Event, Owner, Reference, Seed},
};
use collector_primitives::{
    asymmetric::{FlightGate, FlightGuard},
    first_reason::FirstReason,
    shared_lane::SharedLane,
};
use engine_allocation as allocation;
use engine_intelligence::{intel_source_sites as sites, intel_sources as native};
use engine_native_memory::{Memory, read};
use replay_data::{
    cadence::SKIPPED_REASON,
    rows::{IntelContributionRow, IntelSourceRow},
};
use std::{
    cell::Cell,
    collections::HashMap,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static FLIGHT: FlightGate = FlightGate::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(1);
static ORDER: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
static ORIGINAL: [AtomicUsize; sites::SITES.len()] =
    [const { AtomicUsize::new(0) }; sites::SITES.len()];
static CALLS: [AtomicU64; sites::SITES.len()] = [const { AtomicU64::new(0) }; sites::SITES.len()];
static FAILURES: [AtomicU64; sites::SITES.len()] =
    [const { AtomicU64::new(0) }; sites::SITES.len()];
static FIRST_FAILURE: FirstReason = FirstReason::new();
static CURRENT: AtomicPtr<SharedLane<Event>> = AtomicPtr::new(ptr::null_mut());
static REFERENCES: AtomicPtr<ReferenceIndex> = AtomicPtr::new(ptr::null_mut());
static STORE: Mutex<Store> = Mutex::new(Store::new());

#[derive(Clone, Copy, Default)]
struct Context {
    root: usize,
    pool: u32,
    source: i32,
    pool_data: usize,
    pool_count: u32,
}
thread_local! {static CONTEXT:Cell<Context>=const {Cell::new(Context {root:0,pool:0,source:0,pool_data:0,pool_count:0})};}
struct ContextGuard(Context);
impl Drop for ContextGuard {
    fn drop(&mut self) {
        CONTEXT.with(|context| context.set(self.0));
    }
}
struct Flight {
    guard: Option<FlightGuard<'static>>,
    complete: bool,
}
impl Flight {
    #[inline(always)]
    fn new() -> Self {
        Self {
            guard: Some(FLIGHT.enter(true)),
            complete: false,
        }
    }
}
impl Drop for Flight {
    #[inline(always)]
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            if !self.complete {
                fail("intel source writer did not complete");
                guard.leak_and_retain();
            } else {
                drop(guard);
            }
        }
    }
}

fn fail(reason: &'static str) {
    FIRST_FAILURE.record(reason);
    FAILED.fetch_add(1, Ordering::SeqCst);
}

fn refused(site: usize, reason: &'static str) {
    FAILURES[site].fetch_add(1, Ordering::Relaxed);
    fail(reason);
}

#[derive(Default)]
struct ReferenceIndex {
    addresses: Vec<usize>,
}
impl ReferenceIndex {
    fn contains(&self, address: usize) -> bool {
        self.addresses.binary_search(&address).is_ok()
    }
}

struct Store {
    #[expect(
        clippy::vec_box,
        reason = "Published lane pointers must remain stable when the retention vector grows."
    )]
    lanes: Vec<Box<SharedLane<Event>>>,
    owners: Vec<Owner>,
    owner_by_root: Option<HashMap<usize, usize>>,
    ref_by_addr: Option<HashMap<usize, (usize, usize)>>,
    net_by_addr: Option<HashMap<usize, usize>>,
    events: Vec<(u64, usize, u64, Event)>,
    networks: Vec<Reference>,
    network_seeds: Vec<(u32, u32, usize)>,
    network_due: bool,
    network_refresh: bool,
    network_seen: bool,
    index: Option<Box<ReferenceIndex>>,
    #[expect(
        clippy::vec_box,
        reason = "Retired indexes retain their published addresses until native callbacks quiesce."
    )]
    retired: Vec<Box<ReferenceIndex>>,
    index_scratch: Vec<usize>,
    acquired: Vec<usize>,
    acquired_networks: Vec<usize>,
    dirty: Vec<usize>,
    next_networks: Vec<Reference>,
    epoch: u64,
    failure: u64,
    rejected: u64,
    valid: bool,
    snapshots: [crate::retained_snapshot::RetainedSnapshot; 2],
}
impl Store {

    const fn new() -> Self {
        Self {
            lanes: Vec::new(),
            owners: Vec::new(),
            owner_by_root: None,
            ref_by_addr: None,
            net_by_addr: None,
            events: Vec::new(),
            networks: Vec::new(),
            network_seeds: Vec::new(),
            network_due: false,
            network_refresh: false,
            network_seen: false,
            index: None,
            retired: Vec::new(),
            index_scratch: Vec::new(),
            acquired: Vec::new(),
            acquired_networks: Vec::new(),
            dirty: Vec::new(),
            next_networks: Vec::new(),
            epoch: 0,
            failure: 0,
            rejected: 0,
            valid: false,
            snapshots: [const { crate::retained_snapshot::RetainedSnapshot::new() }; 2],
        }
    }
    fn prepare(&mut self, demand: usize) -> Result<(), &'static str> {
        if self.lanes.last().is_some_and(|lane| lane.len() >= demand) {
            return Ok(());
        }
        let lane = Box::new(
            SharedLane::new(demand, 0).map_err(|_| "intel source event lane allocation failed")?,
        );
        allocation::reserve(&mut self.lanes, 1)
            .map_err(|_| "intel source lane owner allocation failed")?;
        self.lanes.push(lane);
        CURRENT.store(
            std::ptr::from_ref(self.lanes.last().unwrap().as_ref()) as *mut _,
            Ordering::SeqCst,
        );
        Ok(())
    }
    fn rebuild_lookups(&mut self) {
        let owner_map = self.owner_by_root.get_or_insert_with(HashMap::new);
        owner_map.clear();
        if allocation::advisory(|| allocation::reserve_map(owner_map, self.owners.len())).is_ok() {
            for (index, owner) in self.owners.iter().enumerate() {
                owner_map.insert(owner.seed.root, index);
            }
        }
        let ref_map = self.ref_by_addr.get_or_insert_with(HashMap::new);
        ref_map.clear();
        if allocation::advisory(|| allocation::reserve_map(ref_map, self.owners.len() * 3)).is_ok()
        {
            for (index, owner) in self.owners.iter().enumerate() {
                for (position, reference) in owner.references.iter().enumerate() {
                    ref_map.insert(reference.address, (index, position));
                }
            }
        }
        let net_map = self.net_by_addr.get_or_insert_with(HashMap::new);
        net_map.clear();
        if allocation::advisory(|| allocation::reserve_map(net_map, self.networks.len())).is_ok() {
            for (index, reference) in self.networks.iter().enumerate() {
                net_map.insert(reference.address, index);
            }
        }
    }
    fn publish_index(&mut self) -> Result<(), &'static str> {
        self.rebuild_lookups();
        self.index_scratch.clear();
        allocation::reserve(
            &mut self.index_scratch,
            self.owners
                .len()
                .saturating_mul(3)
                .saturating_add(self.networks.len()),
        )
        .map_err(|_| "intel held reference index storage allocation failed")?;
        for owner in &self.owners {
            self.index_scratch
                .extend(owner.references.iter().map(|reference| reference.address));
        }
        self.index_scratch
            .extend(self.networks.iter().map(|reference| reference.address));
        self.index_scratch.sort_unstable();
        self.index_scratch.dedup();
        if self
            .index
            .as_ref()
            .is_some_and(|index| index.addresses == self.index_scratch)
        {
            return Ok(());
        }
        let index = Box::new(ReferenceIndex {
            addresses: std::mem::take(&mut self.index_scratch),
        });
        let pointer = std::ptr::from_ref(index.as_ref()) as *mut _;
        allocation::reserve(&mut self.retired, 1)
            .map_err(|_| "intel retired reference index allocation failed")?;
        if let Some(old) = self.index.replace(index) {
            self.retired.push(old);
        }
        REFERENCES.store(pointer, Ordering::SeqCst);
        if !FLIGHT.any_active() {
            self.retired.clear();
        }
        Ok(())
    }
    fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: Option<&[Seed]>,
        force: bool,
        contributions: &mut Vec<IntelContributionRow>,
        sources: &mut Vec<IntelSourceRow>,
    ) -> Result<(Option<usize>, Option<usize>), &'static str> {
        let Some(revision) = FLIGHT.quiet_epoch() else {
            return Err("intel source writer in progress");
        };
        let epoch = EPOCH.load(Ordering::SeqCst);
        let failure = FAILED.load(Ordering::SeqCst);
        let rejected = self.lanes.iter().map(|lane| lane.rejected()).sum::<u64>();
        if self.epoch != epoch || self.failure != failure || self.rejected != rejected {
            self.valid = false;
        }
        if !ENABLED.load(Ordering::Acquire) {
            self.valid = false;
            return Err("intel source observers disabled");
        }
        let mut events = std::mem::take(&mut self.events);
        events.clear();
        let mut demand = 0;
        for (lane_index, lane) in self.lanes.iter().enumerate() {
            demand = demand.max(lane.required_entries());
            for (order, index) in lane.published(|event| event.order)? {
                allocation::reserve(&mut events, 1)
                    .map_err(|_| "intel source event drain allocation failed")?;
                events.push((order, lane_index, index, lane.record(index).0));
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let result = (|| {
            let bootstrap = force || !self.valid;
            if !self.network_seen {
                return Err("intel source network reference membership unavailable");
            }
            if bootstrap && seeds.is_none() {
                return Err(
                    "intel source state requires existing daily or forced census after lost observation",
                );
            }
            let mut changed = if bootstrap {
                Changed::BOTH
            } else {
                Changed::NONE
            };
            let membership = bootstrap || seeds.is_some() || self.network_due;
            self.acquired.clear();
            self.acquired_networks.clear();
            if let Some(seeds) = seeds {
                allocation::reserve(&mut self.acquired, seeds.len())
                    .map_err(|_| "intel source bootstrap membership allocation failed")?;
                let old_count = self.owners.len();
                self.owners.retain(|owner| seeds.contains(&owner.seed));
                let mut owners_changed = self.owners.len() != old_count;
                for &seed in seeds {
                    let index = if let Some(index) =
                        self.owners.iter().position(|owner| owner.seed == seed)
                    {
                        index
                    } else {
                        allocation::reserve(&mut self.owners, 1)
                            .map_err(|_| "intel source owner storage allocation failed")?;
                        self.owners.push(Owner::default());
                        self.owners.len() - 1
                    };
                    if bootstrap || self.owners[index].seed != seed {
                        self.owners[index].bootstrap(memory, base, seed)?;
                        self.acquired.push(seed.root);
                        owners_changed = true;
                    }
                }
                if owners_changed {
                    self.owners.sort_unstable_by_key(|owner| owner.seed.owner);
                    changed.merge(Changed::BOTH);
                }
            }
            if self.network_due || bootstrap {
                self.next_networks.clear();
                allocation::reserve(&mut self.next_networks, self.network_seeds.len())
                    .map_err(|_| "intel network source membership allocation failed")?;
                allocation::reserve(&mut self.acquired_networks, self.network_seeds.len())
                    .map_err(|_| "intel acquired network membership allocation failed")?;
                for &(owner, target, address) in &self.network_seeds {
                    let held = self.networks.iter().find(|reference| {
                        reference.address == address
                            && reference.owner == owner
                            && reference.target == target
                    });
                    self.next_networks.push(
                        if !bootstrap
                            && !self.network_refresh
                            && let Some(held) = held
                        {
                            *held
                        } else {
                            self.acquired_networks.push(address);
                            Reference {
                                address,
                                owner,
                                target,
                                kind: 3,
                                value: native::reference(memory, base, address)?,
                            }
                        },
                    );
                }
                self.next_networks
                    .sort_unstable_by_key(|reference| (reference.owner, reference.target));
                changed.sources |= self.next_networks != self.networks;
                std::mem::swap(&mut self.networks, &mut self.next_networks);
                self.network_due = false;
                self.network_refresh = false;
            }
            if membership {
                self.publish_index()?;
            }
            if events.is_empty() && !changed.any() {
                return Ok((None, None));
            }
            self.dirty.clear();
            let mut reindex = false;
            allocation::reserve(&mut self.dirty, events.len())
                .map_err(|_| "intel dirty source owner storage allocation failed")?;
            for (_, _, _, event) in &events {
                if event.kind == state::REFERENCE {
                    if self.acquired_networks.contains(&event.object) {
                        continue;
                    }
                    let value = SourceReferenceValue::from_event(*event);
                    let net_idx = self
                        .net_by_addr
                        .as_ref()
                        .and_then(|map| map.get(&event.object).copied())
                        .filter(|&idx| {
                            self.networks
                                .get(idx)
                                .is_some_and(|r| r.address == event.object)
                        })
                        .or_else(|| {
                            self.networks
                                .iter()
                                .position(|reference| reference.address == event.object)
                        });
                    if let Some(idx) = net_idx {
                        let reference = &mut self.networks[idx];
                        changed.sources |= reference.value != value;
                        reference.value = value;
                    } else if let Some((index, position)) = self
                        .ref_by_addr
                        .as_ref()
                        .and_then(|map| map.get(&event.object).copied())
                        .filter(|&(index, position)| {
                            self.owners.get(index).is_some_and(|owner| {
                                owner.references[position].address == event.object
                            })
                        })
                        .or_else(|| {
                            self.owners.iter().enumerate().find_map(|(index, owner)| {
                                owner
                                    .references
                                    .iter()
                                    .position(|reference| reference.address == event.object)
                                    .map(|position| (index, position))
                            })
                        })
                        && !self.acquired.contains(&self.owners[index].seed.root)
                    {
                        let reference = &mut self.owners[index].references[position];
                        changed.sources |= reference.value != value;
                        reference.value = value;
                    }
                    continue;
                }
                if self.acquired.contains(&event.object) {
                    continue;
                }
                let index = self
                    .owner_by_root
                    .as_ref()
                    .and_then(|map| map.get(&event.object).copied())
                    .filter(|&index| {
                        self.owners
                            .get(index)
                            .is_some_and(|owner| owner.seed.root == event.object)
                    })
                    .or_else(|| {
                        self.owners
                            .iter()
                            .position(|owner| owner.seed.root == event.object)
                    })
                    .ok_or("intel source writer root absent from existing census")?;
                changed.merge(self.owners[index].apply(*event)?);
                reindex |= event.kind == state::RESET;
                if !self.dirty.contains(&index) {
                    self.dirty.push(index);
                }
            }
            for &index in &self.dirty {
                changed.merge(self.owners[index].acquire(memory)?);
            }
            if reindex {
                self.publish_index()?;
            }
            if !changed.any() {
                return Ok((None, None));
            }
            let contribution_len = if changed.contributions {
                contributions.clear();
                for owner in &self.owners {
                    owner.contribution_rows(contributions)?;
                }
                Some(contributions.len())
            } else {
                None
            };
            let source_len = if changed.sources {
                sources.clear();
                let mut net_at = 0;
                for owner in &self.owners {
                    let id = owner.seed.owner;
                    while net_at < self.networks.len() && self.networks[net_at].owner < id {
                        allocation::reserve(sources, 1)
                            .map_err(|_| "intel network reference rows allocation failed")?;
                        sources.push(self.networks[net_at].row());
                        net_at += 1;
                    }
                    let net_start = net_at;
                    while net_at < self.networks.len() && self.networks[net_at].owner == id {
                        net_at += 1;
                    }
                    owner.source_rows(sources, &self.networks[net_start..net_at])?;
                }
                if net_at < self.networks.len() {
                    let rest = &self.networks[net_at..];
                    allocation::reserve(sources, rest.len())
                        .map_err(|_| "intel network reference rows allocation failed")?;
                    sources.extend(rest.iter().map(Reference::row));
                }
                Some(sources.len())
            } else {
                None
            };
            Ok((contribution_len, source_len))
        })();
        for (_, lane, index, _) in &events {
            self.lanes[*lane].release(*index);
        }
        for lane in &self.lanes {
            lane.advance(lane.rejected());
        }
        self.events = events;
        self.prepare(demand)?;
        if FLIGHT.quiet_epoch() != Some(revision)
            || EPOCH.load(Ordering::SeqCst) != epoch
            || FAILED.load(Ordering::SeqCst) != failure
        {
            self.valid = false;
            return Err("intel source observation changed during cut");
        }
        self.valid = result.is_ok();
        self.epoch = epoch;
        self.failure = failure;
        self.rejected = rejected;
        result
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Some(lane) = self.lanes.last() {
            let _ = CURRENT.compare_exchange(
                std::ptr::from_ref(lane.as_ref()) as *mut _,
                ptr::null_mut(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
        if let Some(index) = &self.index {
            let _ = REFERENCES.compare_exchange(
                std::ptr::from_ref(index.as_ref()) as *mut _,
                ptr::null_mut(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
    }
}

struct SourceReferenceValue;
impl SourceReferenceValue {
    fn from_event(event: Event) -> native::SourceReference {
        native::SourceReference {
            country: event.values[0] as u32,
            pool: event.values[1] as i32,
            id: event.values[2] as i32,
            lifetime: event.values[3] as u32,
        }
    }
}

pub(crate) fn network_membership(
    rows: &[engine_intelligence::ccountry_intel_network::Summary],
    lifetime_changed: bool,
) -> Result<(), &'static str> {
    let mut store = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    store.network_seen = false;
    store.network_seeds.clear();
    allocation::reserve(&mut store.network_seeds, rows.len())
        .map_err(|_| "intel network reference seed storage allocation failed")?;
    for row in rows {
        store.network_seeds.push((
            row.owner_country,
            row.target_country,
            row.native_pointer
                .checked_add(engine_intelligence::ccountry_intel_network::STATIC_SOURCE_REFERENCE)
                .ok_or("intel network reference address overflow")?,
        ));
    }
    store.network_due = true;
    store.network_refresh |= lifetime_changed;
    store.network_seen = true;
    Ok(())
}

pub(crate) fn network_refused() {
    let mut store = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    store.network_seen = false;
    store.valid = false;
}

pub(crate) fn capture(
    memory: &impl Memory,
    base: usize,
    seeds: Option<&[Seed]>,
    force: bool,
    contributions: &mut Vec<IntelContributionRow>,
    sources: &mut Vec<IntelSourceRow>,
    shared: &mut [Option<std::sync::Arc<crate::retained_snapshot::Snapshot>>],
) -> (engine_native_memory::walk::Outcome, engine_native_memory::walk::Outcome) {
    use engine_native_memory::walk::Outcome;
    let mut store = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (left, right) = match store.capture(memory, base, seeds, force, contributions, sources) {
        Ok((left, right)) => (
            left.map_or(
                Outcome::Unavailable {
                    reason: SKIPPED_REASON,
                },
                |len| Outcome::Rows { len },
            ),
            right.map_or(
                Outcome::Unavailable {
                    reason: SKIPPED_REASON,
                },
                |len| Outcome::Rows { len },
            ),
        ),
        Err(reason) => {
            store.valid = false;
            (
                Outcome::Unavailable { reason },
                Outcome::Unavailable { reason },
            )
        }
    };
    let left = store.snapshots[0].publish(
        replay_data::rows::TableId::IntelContributions,
        left,
        || crate::retained_snapshot::AcquiredRows::Contributions(std::mem::take(contributions)),
        &mut shared[0],
    );
    let right = store.snapshots[1].publish(
        replay_data::rows::TableId::IntelSources,
        right,
        || crate::retained_snapshot::AcquiredRows::Sources(std::mem::take(sources)),
        &mut shared[1],
    );
    if let Some(reason) = [left, right].into_iter().find_map(|outcome| match outcome {
        Outcome::Unavailable { reason } if reason != SKIPPED_REASON => Some(reason),
        _ => None,
    }) {
        store.valid = false;
        for snapshot in &mut store.snapshots {
            snapshot.clear();
        }
        shared.fill(None);
        return (
            Outcome::Unavailable { reason },
            Outcome::Unavailable { reason },
        );
    }
    (left, right)
}
pub fn boundary(invalidate: bool) {
    if invalidate {
        EPOCH.fetch_add(1, Ordering::SeqCst);
    }
}
pub fn active() -> usize {
    FLIGHT.active()
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "lost": FAILED.load(Ordering::Relaxed),
        "first_loss": FIRST_FAILURE.get(),
        "failed_sites": crate::status::map(
            sites::SITES
                .iter()
                .map(|site| site.name)
                .zip(FAILURES.iter().map(|count| count.load(Ordering::Relaxed)))
                .filter(|(_, count)| *count != 0),
        ),
    })
}

fn publish(mut event: Event) -> Result<(), &'static str> {
    event.order = ORDER.fetch_add(1, Ordering::SeqCst);
    unsafe { CURRENT.load(Ordering::SeqCst).as_ref() }
        .ok_or("intel source event lane not prepared")?
        .reserve()
        .ok_or("intel source event allocation failed")?
        .publish(event);
    Ok(())
}

unsafe extern "system-unwind" fn callback<const I: usize>(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    let mut flight = Flight::new();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::IntelSources);
    CALLS[I].fetch_add(1, Ordering::Relaxed);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let previous = CONTEXT.with(Cell::get);
    let setup = (|| {
        let context = match I {
            5 => Context {
                root: a,
                pool: c as u32,
                ..Default::default()
            },
            6 => Context {
                root: a,
                pool: sites::CHEAT_POOL,
                ..Default::default()
            },
            7 | 8 => {
                let reference = native::reference(&memory, BASE.load(Ordering::Acquire), b)?;
                Context {
                    root: a,
                    pool: reference.pool as u32,
                    source: reference.id,
                    ..Default::default()
                }
            }
            9 => Context {
                root: a,
                pool: b as u32,
                ..Default::default()
            },
            10 | 11 => {
                let pools = native::array(
                    &memory,
                    a,
                    native::DYNAMIC_POOLS,
                    native::DYNAMIC_POOL_STRIDE,
                )?;
                Context {
                    root: a,
                    pool_data: pools.data,
                    pool_count: pools.count,
                    ..Default::default()
                }
            }
            _ => previous,
        };
        CONTEXT.with(|slot| slot.set(context));
        let input = if I == 15 || I == 18 {
            native::lane_values(&memory, c)?
        } else {
            [0; 4]
        };
        Ok::<_, &'static str>((context, input))
    })();
    let _context = ContextGuard(previous);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[I].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d) });
    let post = (|| {
        let (context, input) = setup?;
        let mut event = Event {
            object: a,
            ..Default::default()
        };
        match I {
            0 => {
                event.kind = state::RESET;
                event.values[0] = i64::from(
                    native::array(&memory, a, native::STATIC_POOLS, native::STATIC_POOL_STRIDE)?
                        .count,
                );
                event.values[1] = i64::from(
                    native::array(
                        &memory,
                        a,
                        native::DYNAMIC_POOLS,
                        native::DYNAMIC_POOL_STRIDE,
                    )?
                    .count,
                );
            }
            1 => event.kind = state::ALLIED_REPLACE,
            23 => event.kind = state::ALLIED_CLEAR,
            2 => event.kind = state::CRYPTOLOGY_REPLACE,
            3 => event.kind = state::CONTRIBUTIONS_REPLACE,
            4 => event.kind = state::PRIOR_ROTATE,
            12 => event.kind = state::STATIC_PRUNE,
            13 => {
                event.kind = state::STATIC_CLEAR;
                event.pool = b as u32;
            }
            14 => {
                event.kind = state::DYNAMIC_CLEAR;
                event.pool = b as u32;
            }
            15..=20 => {
                if context.root == 0 {
                    return Err("intel source low-level writer lacks owner context");
                }
                event.object = context.root;
                event.pool = context.pool;
                event.source = context.source;
                match I {
                    15 => {
                        event.kind = state::STATIC_SET;
                        event.target = b as i32;
                        event.values = input;
                    }
                    16 => event.kind = state::STATIC_REMOVE,
                    17 => {
                        event.kind = state::STATIC_CREATE;
                        event.source = result as i32;
                    }
                    18 => {
                        event.kind = state::DYNAMIC_ADD;
                        event.target = b as i32;
                        event.values = input;
                        if input.iter().all(|value| *value <= 0) {
                            return Ok(());
                        }
                    }
                    19 | 20 => {
                        let offset = a
                            .checked_sub(context.pool_data)
                            .ok_or("intel dynamic writer precedes pool span")?;
                        if offset % native::DYNAMIC_POOL_STRIDE != 0
                            || offset / native::DYNAMIC_POOL_STRIDE >= context.pool_count as usize
                        {
                            return Err("intel dynamic writer outside actual pool span");
                        }
                        event.pool = (offset / native::DYNAMIC_POOL_STRIDE) as u32;
                        if I == 19 {
                            event.kind = state::DYNAMIC_AGGREGATE;
                        } else {
                            event.kind = state::DYNAMIC_DECAY;
                            event.values = [b as i64, c as i64, 0, 0];
                        }
                    }
                    _ => unreachable!(),
                }
            }
            21 | 22 => {
                let index = REFERENCES.load(Ordering::SeqCst);
                if let Some(index) = unsafe { index.as_ref() } {
                    for address in if I == 21 { [a, b] } else { [a, 0] } {
                        if address != 0 && index.contains(address) {
                            let value =
                                native::reference(&memory, BASE.load(Ordering::Acquire), address)?;
                            publish(Event {
                                object: address,
                                kind: state::REFERENCE,
                                values: [
                                    i64::from(value.country),
                                    i64::from(value.pool),
                                    i64::from(value.id),
                                    i64::from(value.lifetime),
                                ],
                                ..Default::default()
                            })?;
                        }
                    }
                }
                return Ok(());
            }
            _ => return Ok(()),
        }
        publish(event)
    })();
    if let Err(reason) = post {
        refused(I, reason);
    }
    flight.complete = true;
    result
}

static BASE: AtomicUsize = AtomicUsize::new(0);
pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The host authenticates the image and retains hooks and stores until
    /// restoration and native callback quiescence are both proved.
    pub unsafe fn new(
        memory: &impl Memory,
        base: usize,
        capture: bool,
    ) -> Result<Self, String> {
        if !capture {
            return Ok(Self { hooks: Vec::new() });
        }
        BASE.store(base, Ordering::Release);
        let game = engine_sites::resolve(base, engine_native::cgame_state::SINGLETON_RVA)
            .ok()
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("intel source bootstrap game state unreadable")?;
        let countries = if game == 0 {
            0
        } else {
            engine_native_memory::cpdx_array::Array::read(
                memory,
                game + engine_native::cgame_state::COUNTRIES,
                8,
            )
            .ok_or("intel source bootstrap country header invalid")?
            .count as usize
        };
        STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prepare(
                countries
                    .checked_mul(sites::SITES.len())
                    .ok_or("intel source lane demand overflow")?,
            )?;
        let mut hooks = Vec::new();
        macro_rules! callbacks {($($index:literal),*)=>{[$(callback::<$index> as *const () as usize),*]};}
        for (index, callback) in callbacks!(
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23
        )
        .into_iter()
        .enumerate()
        {
            let site = &sites::SITES[index];
            let (address, original) =
                hook_errors::locate(base, site.rva, "intel sources", site.name, index)?;
            let hook = unsafe { Hook::new(address, callback, original) }.map_err(|cause| {
                hook_errors::site(
                    "intel sources",
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
                "intel sources",
                &self.hooks,
                sites::SITES.iter().map(|site| site.name),
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
        REFERENCES.store(ptr::null_mut(), Ordering::SeqCst);
        *STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Store::new();
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("certified source derivation read native memory")
        }
    }

    #[test]
    fn intel_sources_owners_carry_without_reads_or_allocations_and_loss_requires_census() {
        ENABLED.store(true, Ordering::SeqCst);
        let mut store = Store::new();
        store.prepare(2).unwrap();
        store.owners = (1..=5)
            .map(|owner| {
                let mut state = Owner::default();
                state.seed = Seed {
                    owner,
                    root: owner as usize,
                    country: 0,
                };
                state
            })
            .collect();
        store.network_seen = true;
        store.valid = true;
        store.epoch = EPOCH.load(Ordering::SeqCst);
        store.failure = FAILED.load(Ordering::SeqCst);
        let mut contributions = Vec::new();
        let mut sources = Vec::new();
        let result = crate::test_support::with_allocations_denied(|| {
            store.capture(&NoRead, 0, None, false, &mut contributions, &mut sources)
        });
        assert_eq!(result.unwrap(), (None, None));
        assert!(contributions.is_empty());
        assert!(sources.is_empty());
        let lane = store.lanes.last().unwrap();
        let burst = 4 * lane.len() as u64 + 3;
        for order in 1..=burst {
            lane.reserve().unwrap().publish(Event {
                order,
                object: 5,
                kind: state::STATIC_PRUNE,
                ..Default::default()
            });
        }
        assert_eq!(
            store
                .capture(&NoRead, 0, None, false, &mut contributions, &mut sources)
                .unwrap(),
            (None, None)
        );
        assert_eq!(
            store.events.iter().map(|event| event.0).collect::<Vec<_>>(),
            (1..=burst).collect::<Vec<_>>()
        );
        assert!(store.lanes.last().unwrap().len() >= burst as usize);
        assert_eq!(store.rejected, 0);
        let lane = store.lanes.last().unwrap();
        assert!(ptr::eq(CURRENT.load(Ordering::SeqCst), &raw const **lane));
        for _ in 0..lane.len() {
            lane.try_reserve().unwrap().publish(Event {
                object: 5,
                kind: state::STATIC_PRUNE,
                ..Default::default()
            });
        }
        let site = 12;
        let reason = crate::test_support::with_allocations_denied(|| {
            publish(Event {
                object: 5,
                kind: state::STATIC_PRUNE,
                ..Default::default()
            })
        })
        .unwrap_err();
        assert_eq!(reason, "intel source event allocation failed");
        refused(site, reason);
        let status = serde_json::to_value(status()).unwrap();
        assert_eq!(status["first_loss"], "intel source event allocation failed");
        assert_eq!(status["failed_sites"][sites::SITES[site].name], 1);
        assert!(
            store
                .capture(&NoRead, 0, None, false, &mut contributions, &mut sources)
                .unwrap_err()
                .contains("requires existing daily or forced census")
        );
        let mut world = engine_native::ccountry::synthetic::World::new(0, 0, 0x200);
        let address = world.alloc(24);
        world.u64(address, native::REFERENCE_VTABLE.0 as u64);
        world.u32(address + 8, 19);
        world.u32(address + 12, 7);
        world.u32(address + 16, 23);
        world.u32(address + 20, 29);
        store.network_seeds.push((17, 19, address));
        store.network_due = true;
        store
            .lanes
            .last()
            .unwrap()
            .reserve()
            .unwrap()
            .publish(Event {
                kind: state::REFERENCE,
                object: address,
                values: [0; 4],
                ..Default::default()
            });
        store
            .capture(
                &world.memory(),
                0,
                Some(&[]),
                true,
                &mut contributions,
                &mut sources,
            )
            .unwrap();
        let row = sources.iter().find(|row| row.source == 3).unwrap();
        assert_eq!(
            (
                row.reference_country,
                row.reference_pool,
                row.id,
                row.discriminant
            ),
            (19, 7, 23, 29)
        );
        assert_eq!(
            store
                .capture(&NoRead, 0, None, false, &mut contributions, &mut sources)
                .unwrap(),
            (None, None)
        );
        world.u32(address + 16, 31);
        store.network_due = true;
        store.network_refresh = true;
        store
            .lanes
            .last()
            .unwrap()
            .reserve()
            .unwrap()
            .publish(Event {
                kind: state::REFERENCE,
                object: address,
                values: [19, 7, 23, 29],
                ..Default::default()
            });
        let (contrib_out, source_out) = store
            .capture(
                &world.memory(),
                0,
                None,
                false,
                &mut contributions,
                &mut sources,
            )
            .unwrap();
        assert_eq!(contrib_out, None);
        assert!(source_out.is_some());
        assert_eq!(sources.iter().find(|row| row.source == 3).unwrap().id, 31);
        ENABLED.store(false, Ordering::SeqCst);
    }
}
