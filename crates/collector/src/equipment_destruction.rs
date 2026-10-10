//! Destruction writers only. Every event uses its native owner and exact signed
//! pool difference. Try-locks never wait on a game thread; overlap, refusal,
//! overflow or a lost drain refuses its interval until a clean hourly boundary.
use crate::hook::Hook;
type Clock = engine::cgame_state::Clock<u64>;
use engine::{Memory, equipment_destruction as native, equipment_entry};
use native::SiteId;
mod queue_index;
mod transfers;
use queue_index::FixedIndex;
use std::{
    borrow::Cow,
    cell::Cell,
    ops::Range,
    sync::{
        LazyLock, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use transfers::{Credits, Net, PoolState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub country: u32,
    pub cause: native::Cause,
    pub disposition: native::Disposition,
    pub expeditionary_owner: u32,
    /// Nonzero only for the once-per-division deletion witness. Equipment
    /// removals remain aggregated by owner, cause and complete design handle.
    pub division_handle: u64,
    pub variant: u64,
    /// Signed fixed5 net removal; a negative amount is a native pool gain.
    pub amount_raw: i64,
}

#[derive(Clone, Copy, serde::Serialize)]
struct Refusal {
    site_index: Option<usize>,
    reason: &'static str,
}

fn record_refusal(site_index: Option<usize>, reason: &'static str) -> Refusal {
    let refusal = Refusal { site_index, reason };
    let _ = FIRST_REFUSAL.set(refusal);
    if let Some(site) = site_index {
        SITE_REFUSALS[site].fetch_add(1, Ordering::Relaxed);
    }
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    poison();
    refusal
}

#[derive(Clone, Copy, Default)]
struct HeldRow {
    destination: usize,
    country: u32,
    row: equipment_entry::Row,
}
#[derive(Clone, Copy, Default)]
struct MergeReserve {
    pool: usize,
    country: u32,
    identity: u32,
    held_start: usize,
    held_end: usize,
}

struct Store {
    _charge: crate::raw_memory::Charge,
    entries: Vec<Entry>,
    queued: FixedIndex<u64, Queued>,
    queued_by_army: FixedIndex<usize, u64>,
    queued_limit: usize,
    queued_count: usize,
    before: Vec<equipment_entry::Row>,
    after: Vec<equipment_entry::Row>,
    raw: Vec<u8>,
    net: Vec<Net>,
    armies: Vec<usize>,
    held: Vec<HeldRow>,
    reserves: Vec<MergeReserve>,
    credits: Credits,
    transition: bool,
    refusal: Option<Refusal>,
    interval_refusal: Option<Refusal>,
    attach_interval: bool,
    unproven_queue: bool,
}
impl Store {
    fn new() -> Self {
        Self {
            _charge: Default::default(),
            entries: Vec::new(),
            queued: FixedIndex::new(),
            queued_by_army: FixedIndex::new(),
            queued_limit: 0,
            queued_count: 0,
            before: Vec::new(),
            after: Vec::new(),
            raw: Vec::new(),
            net: Vec::new(),
            armies: Vec::new(),
            held: Vec::new(),
            reserves: Vec::new(),
            credits: Credits::new(),
            transition: true,
            refusal: None,
            interval_refusal: None,
            attach_interval: true,
            unproven_queue: false,
        }
    }
    #[cfg(test)]
    fn owned_capacity(&self) -> crate::raw_memory::Capacity {
        owned_capacity(
            &self.entries,
            &self.before,
            &self.after,
            &self.raw,
            &self.queued,
            &self.queued_by_army,
            (
                &self.net,
                &self.armies,
                &self.held,
                &self.reserves,
                &self.credits,
            ),
        )
    }
    fn append(&mut self, site: SiteId, entry: Entry) -> Result<(), &'static str> {
        append_entry(&mut self.entries, site, entry)
    }
    fn refuse(&mut self, site_index: Option<usize>, reason: &'static str) {
        self.refusal
            .get_or_insert(record_refusal(site_index, reason));
    }
    fn boundary(&mut self, transition: bool, load: bool) {
        EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
        CALL_ATTEMPTS.store(0, Ordering::Relaxed);
        self.transition = transition;
        let refused = POISONED.swap(false, Ordering::SeqCst);
        self.interval_refusal = None;
        if transition {
            self.entries.clear();
            self.refusal = None;
            self.unproven_queue = false;
            self.attach_interval |= refused;
            if !load && self.credits.has_unsettled() {
                self.refuse(None, "equipment returned source lifetime incomplete");
            }
        } else if refused {
            self.entries.clear();
            self.unproven_queue = false;
            self.attach_interval = true;
            self.interval_refusal = Some(self.refusal.take().unwrap_or(Refusal {
                site_index: None,
                reason: "equipment destruction lost observation; interval unavailable",
            }));
        } else {
            self.refusal = None;
        }
        if load {
            if let Err(reason) = self
                .queued
                .clear()
                .and_then(|()| self.queued_by_army.clear())
            {
                self.refuse(None, reason);
                return;
            }
            if let Err(reason) = self.credits.clear() {
                self.refuse(None, reason);
                return;
            }
            self.queued_count = 0;
            self.unproven_queue = false;
        }
    }
}

fn append_entry(entries: &mut Vec<Entry>, site: SiteId, entry: Entry) -> Result<(), &'static str> {
    if entry.division_handle != 0 {
        if entry.variant != 0 || entry.amount_raw != 0 {
            return Err("equipment deletion witness has equipment amount");
        }
    } else if entry.amount_raw == 0 {
        return Ok(());
    }
    // Pool differences can exceed a census allowance
    // through repeated calls. Retain every reached row, including refusals.
    let attempted = EVENT_ATTEMPTS
        .fetch_add(1, Ordering::Relaxed)
        .saturating_add(1);
    EVENT_REQUIRED.fetch_max(attempted, Ordering::Relaxed);
    if entries.len() == entries.capacity() {
        return Err(event_overflow(site));
    }
    entries.push(entry); // capacity checked; never allocates
    EVENTS.fetch_add(1, Ordering::Relaxed);
    record_hourly_peak(entries.len(), entries.capacity());
    Ok(())
}

// Pending credits already carry compact per-design deficits. This visitor
// settles them without touching scratch borrowed by an outer whole Merge.
fn settle_credit(store: &mut Store, context: Deletion) -> Result<(), &'static str> {
    store
        .credits
        .state(context.handle, context.army, context.country)?;
    let entries = &mut store.entries;
    store
        .credits
        .visit(context.handle, context.army, context.country, |value| {
            let amount_raw = i64::try_from(value.amount)
                .map_err(|_| "equipment deletion correction exceeds signed width")?;
            append_entry(
                entries,
                SiteId::DivisionFree,
                Entry {
                    country: context.country,
                    cause: context.cause,
                    disposition: native::Disposition::Loss,
                    expeditionary_owner: 0,
                    division_handle: 0,
                    variant: value.variant,
                    amount_raw,
                },
            )
        })
}

fn owned_capacity(
    entries: &Vec<Entry>,
    before: &Vec<equipment_entry::Row>,
    after: &Vec<equipment_entry::Row>,
    raw: &Vec<u8>,
    queued: &FixedIndex<u64, Queued>,
    by_army: &FixedIndex<usize, u64>,
    (net, armies, held, reserves, credits): (
        &Vec<Net>,
        &Vec<usize>,
        &Vec<HeldRow>,
        &Vec<MergeReserve>,
        &Credits,
    ),
) -> crate::raw_memory::Capacity {
    use crate::raw_memory::{Capacity, vector_bytes};
    Capacity {
        payload_vectors: vector_bytes(entries),
        auxiliary_vectors: vector_bytes(before)
            + vector_bytes(after)
            + vector_bytes(raw)
            + vector_bytes(net)
            + vector_bytes(armies)
            + vector_bytes(held)
            + vector_bytes(reserves),
        index_vectors: queued.allocated_bytes() as u64
            + by_army.allocated_bytes() as u64
            + credits.allocated_bytes() as u64,
        ..Default::default()
    }
}

/// Off-thread after final drain and hook/IP/callback quiescence.
pub fn release() -> Result<(), &'static str> {
    if ENABLED.load(Ordering::SeqCst)
        || SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst)
        || ACTIVE
            .iter()
            .any(|active| active.load(Ordering::SeqCst) != 0)
        || OBSERVING.load(Ordering::SeqCst)
        || BOUNDARY_ADMISSION.load(Ordering::SeqCst) != 0
    {
        return Err("equipment event storage release before callback quiescence");
    }
    let storage = {
        let mut store = STORE.lock().unwrap_or_else(|error| error.into_inner());
        CAPACITY.store(0, Ordering::Release);
        OFFERED_CAPACITY.store(0, Ordering::Release);
        QUEUED_CAPACITY.store(0, Ordering::Release);
        DESIGNS.store(0, Ordering::Release);
        POOL_CAPACITY.store(0, Ordering::Release);
        EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
        CALL_ATTEMPTS.store(0, Ordering::Relaxed);
        EVENT_REQUIRED.store(0, Ordering::Relaxed);
        POOL_REQUIRED.store(0, Ordering::Relaxed);
        std::mem::replace(&mut *store, Store::new())
    };
    drop(storage);
    let mut swap = STORAGE_SWAP
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    drop(swap.ready.take());
    drop(swap.retired.take());
    Ok(())
}

#[derive(Clone, Copy)]
struct Queued {
    army: usize,
    handle: u64,
    cause: native::Cause,
    disposition: native::Disposition,
}

#[derive(Clone, Copy)]
struct Deletion {
    army: usize,
    handle: u64,
    country: u32,
    cause: native::Cause,
    disposition: native::Disposition,
    expeditionary_owner: u32,
    returned: bool,
    observed: bool,
    ignored: bool,
    // Direct Merge caller remains known when queued HQ disposition wins.
    merge_source: bool,
}
impl Deletion {
    fn ignored(army: usize) -> Self {
        Self {
            army,
            handle: 0,
            country: 0,
            cause: native::Cause::Unknown,
            disposition: native::Disposition::Loss,
            expeditionary_owner: 0,
            returned: false,
            observed: false,
            ignored: true,
            merge_source: false,
        }
    }
}
thread_local! {
    // Unit deletion can enter nested cleanup. Each scope restores its caller's
    // context without allocating or holding STORE across a native callback.
    static DELETION: Cell<Option<Deletion>> = const { Cell::new(None) };
    static WHOLE_MERGE: Cell<bool> = const { Cell::new(false) };
}
struct DeletionScope(Option<Deletion>);
impl DeletionScope {
    fn enter(context: Option<Deletion>) -> Self {
        Self(DELETION.replace(context))
    }
}
impl Drop for DeletionScope {
    fn drop(&mut self) {
        DELETION.set(self.0);
    }
}
static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::new()));
#[derive(Default)]
struct StorageSwap {
    ready: Option<(Store, usize)>,
    retired: Option<Store>,
}
// Native writers never take this slot. Retired vectors are freed off-thread.
static STORAGE_SWAP: LazyLock<Mutex<StorageSwap>> =
    LazyLock::new(|| Mutex::new(StorageSwap::default()));
// The first attach-level diagnostic stays visible while a writer owns STORE.
static FIRST_REFUSAL: OnceLock<Refusal> = OnceLock::new();
// Only the hourly boundary writes this clock. It advances even if a writer
// owns STORE, so an overlapping drain cannot masquerade as a later branch.
static CLOCK: Mutex<Option<Clock>> = Mutex::new(None);
static WORLD_GENERATION: AtomicU64 = AtomicU64::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static OFFERED_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static EVENT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static CALL_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static EVENT_REQUIRED: AtomicUsize = AtomicUsize::new(0);
static POOL_REQUIRED: AtomicUsize = AtomicUsize::new(0);
static QUEUED_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static DESIGNS: AtomicUsize = AtomicUsize::new(0);
static POOL_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static HOURLY_PEAK_EVENTS: AtomicUsize = AtomicUsize::new(0);
static HOURLY_PEAK_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static SUPPLEMENTAL_ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
static OBSERVING: AtomicBool = AtomicBool::new(false);
// Bit 0 owns the boundary, bit 1 remembers a callback entering that boundary,
// and each callback/observation holds four units across its native original.
static BOUNDARY_ADMISSION: AtomicUsize = AtomicUsize::new(0);
static PENDING_BOUNDARY: AtomicBool = AtomicBool::new(false);
static PENDING_LOAD: AtomicBool = AtomicBool::new(false);
// Only refused callbacks advance this fallback. When STORE is unavailable,
// no old head can be silently certified after an unknown native mutation.
static CREDIT_REVISION: AtomicU64 = AtomicU64::new(0);
static SKIPPED_QUEUE: AtomicBool = AtomicBool::new(false);
static ACTIVE: [AtomicUsize; native::SITES.len()] =
    [const { AtomicUsize::new(0) }; native::SITES.len()];
static ORIGINAL: [AtomicUsize; native::SITES.len()] =
    [const { AtomicUsize::new(0) }; native::SITES.len()];
static INSTALLED: [AtomicBool; native::SITES.len()] =
    [const { AtomicBool::new(false) }; native::SITES.len()];
static SITE_ENABLED: [AtomicBool; native::SITES.len()] =
    [const { AtomicBool::new(false) }; native::SITES.len()];
static CALLS: [AtomicU64; native::SITES.len()] = [const { AtomicU64::new(0) }; native::SITES.len()];
static UNHOOKED: OnceLock<Cow<'static, str>> = OnceLock::new();
static SUPPLEMENTAL_UNHOOKED: OnceLock<Cow<'static, str>> = OnceLock::new();
static SITE_REFUSALS: [AtomicU64; native::SITES.len()] =
    [const { AtomicU64::new(0) }; native::SITES.len()];
static EVENTS: AtomicU64 = AtomicU64::new(0);
static OVERFLOW: AtomicU64 = AtomicU64::new(0);
static SITE_OVERFLOW: [AtomicU64; native::SITES.len()] =
    [const { AtomicU64::new(0) }; native::SITES.len()];
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
static DRAINS: AtomicU64 = AtomicU64::new(0);
static HANDED_OVER: AtomicU64 = AtomicU64::new(0);
static OVERLAPS: AtomicU64 = AtomicU64::new(0);
static PREPARE_FAILURES: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_CALLERS: AtomicU64 = AtomicU64::new(0);

pub fn active() -> usize {
    ACTIVE.iter().map(|v| v.load(Ordering::SeqCst)).sum()
}
static EVENT_HINT: AtomicUsize = AtomicUsize::new(0);
pub(crate) fn demand() -> usize {
    EVENT_HINT
        .load(Ordering::Acquire)
        .max(EVENT_REQUIRED.load(Ordering::Acquire))
}
pub fn capacity() -> usize {
    CAPACITY
        .load(Ordering::Acquire)
        .max(OFFERED_CAPACITY.load(Ordering::Acquire))
}
fn record_hourly_peak(events: usize, capacity: usize) {
    if events > HOURLY_PEAK_EVENTS.load(Ordering::Relaxed) {
        // All producers hold STORE, so the capacity belongs to this peak.
        // Partial and refused intervals remain included as headroom evidence.
        HOURLY_PEAK_CAPACITY.store(capacity, Ordering::Relaxed);
        HOURLY_PEAK_EVENTS.store(events, Ordering::Relaxed);
    }
}
fn count_cause(cause: native::Cause) -> native::Cause {
    if cause == native::Cause::Unknown {
        UNKNOWN_CALLERS.fetch_add(1, Ordering::Relaxed);
    }
    cause
}
fn poison() {
    POISONED.store(true, Ordering::SeqCst);
}
fn revoke_active_observation() {
    if OBSERVING.load(Ordering::SeqCst) {
        let _ = CREDIT_REVISION.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_add(1)
        });
    }
}
pub fn lost_drain() {
    LOST.fetch_add(1, Ordering::Relaxed);
    poison();
}

/// Allocate only before enabling, or on the spool thread. Grow from live
/// country, division and saved design censuses, per native event family. These
/// are admission allowances, not per-hour native bounds: repeat bursts can
/// overflow and then explicitly withhold the table. Build new storage before
/// publishing a spare; install it only at an empty, writer-free boundary.
/// A failed sizing observation skips growth and counts a prepare failure; it
/// cannot lose a recorded event or invalidate the existing branch observation.
pub fn prepare(memory: &impl Memory, base: usize, countries: usize) {
    engine::allocation::advisory(|| prepare_storage(memory, base, countries));
}

fn prepare_storage(memory: &impl Memory, base: usize, countries: usize) {
    let (prepared_events, prepared_queue, prepared_designs, prepared_pool) = {
        let mut swap = STORAGE_SWAP
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drop(swap.retired.take());
        swap.ready
            .as_ref()
            .map_or((0, 0, 0, 0), |(store, designs)| {
                (
                    EVENT_HINT.load(Ordering::Acquire),
                    store.queued_limit,
                    *designs,
                    store.before.len(),
                )
            })
    };
    let Some(designs) = native::design_count(memory, base) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(population) = native::population(memory, base) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(events) = log_capacity(countries, population, designs) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    EVENT_HINT.fetch_max(events, Ordering::Release);
    let events = demand();
    // Merge main/held pools can create corresponding reserve rows in
    // addition to S existing entries.
    let Some(pool_bound) = population
        .division_pool_entries
        .checked_add(population.delivery_pool_entries)
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(population.stockpile_entries))
    else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let pool_bound = pool_bound
        .max(POOL_REQUIRED.load(Ordering::Relaxed))
        .max(designs)
        .max(POOL_CAPACITY.load(Ordering::Acquire))
        .max(prepared_pool);
    let queued_bound = population.divisions;
    if events <= capacity().max(prepared_events)
        && designs <= DESIGNS.load(Ordering::Acquire).max(prepared_designs)
        && queued_bound <= QUEUED_CAPACITY.load(Ordering::Acquire).max(prepared_queue)
        && pool_bound <= POOL_CAPACITY.load(Ordering::Acquire).max(prepared_pool)
    {
        return;
    }
    let events = events.max(prepared_events);
    let designs = designs
        .max(DESIGNS.load(Ordering::Acquire))
        .max(prepared_designs);
    let queued_bound = queued_bound
        .max(QUEUED_CAPACITY.load(Ordering::Acquire))
        .max(prepared_queue);
    let Some(raw_bytes) = pool_bound.checked_mul(equipment_entry::STRIDE) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut entries = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut raw = Vec::new();
    let mut net = Vec::new();
    let mut armies = Vec::new();
    let mut held = Vec::new();
    let mut reserves = Vec::new();
    let Some(net_bound) = pool_bound.checked_mul(3) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(army_bound) = queued_bound.checked_add(1) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let credits = Credits::try_new(queued_bound, pool_bound);
    let indexes = FixedIndex::try_new(queued_bound)
        .and_then(|queued| FixedIndex::try_new(queued_bound).map(|by_army| (queued, by_army)));
    if engine::allocation::reserve_exact(&mut entries, events).is_err()
        || indexes.is_err()
        || credits.is_err()
        || engine::allocation::reserve_exact(&mut net, net_bound).is_err()
        || engine::allocation::reserve_exact(&mut armies, army_bound).is_err()
        || engine::allocation::reserve_exact(&mut before, pool_bound).is_err()
        || engine::allocation::reserve_exact(&mut after, pool_bound).is_err()
        || engine::allocation::reserve_exact(&mut held, pool_bound).is_err()
        || engine::allocation::reserve_exact(&mut reserves, army_bound).is_err()
        || engine::allocation::reserve_exact(&mut raw, raw_bytes).is_err()
    {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let Ok((queued, queued_by_army)) = indexes else {
        return;
    };
    let Ok(credits) = credits else {
        return;
    };
    before.resize(pool_bound, equipment_entry::Row::default());
    after.resize(pool_bound, equipment_entry::Row::default());
    held.resize(pool_bound, HeldRow::default());
    reserves.resize(army_bound, MergeReserve::default());
    net.resize(net_bound, Net::default());
    armies.resize(army_bound, 0);
    raw.resize(raw_bytes, 0);
    let mut charge = crate::raw_memory::Charge::default();
    charge.set(owned_capacity(
        &entries,
        &before,
        &after,
        &raw,
        &queued,
        &queued_by_army,
        (&net, &armies, &held, &reserves, &credits),
    ));
    let storage = Store {
        _charge: charge,
        entries,
        queued,
        queued_by_army,
        queued_limit: queued_bound,
        before,
        after,
        raw,
        net,
        armies,
        held,
        reserves,
        credits,
        ..Store::new()
    };
    let offered = events;
    STORAGE_SWAP
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .ready = Some((storage, designs));
    // The spool sizes receiving frames before the boundary installs this spare.
    OFFERED_CAPACITY.fetch_max(offered, Ordering::Release);
}

// Caller owns boundary admission and STORE; no native reads or vector drops.
fn install_prepared(store: &mut Store) {
    if !store.entries.is_empty() {
        return;
    }
    let Ok(mut swap) = STORAGE_SWAP.try_lock() else {
        return;
    };
    if swap.retired.is_some() {
        return;
    }
    let Some((ready, designs)) = swap.ready.as_mut() else {
        return;
    };
    let copy = ready
        .queued
        .clear()
        .and_then(|()| ready.queued_by_army.clear())
        .and_then(|()| ready.credits.clear())
        .and_then(|()| {
            store
                .queued
                .iter()
                .try_for_each(|(&key, &value)| ready.queued.insert(key, value))
        })
        .and_then(|()| {
            store
                .queued_by_army
                .iter()
                .try_for_each(|(&key, &value)| ready.queued_by_army.insert(key, value))
        });
    if let Err(reason) = copy.and_then(|()| store.credits.copy_to(&mut ready.credits)) {
        store.refuse(None, reason);
        return;
    }
    std::mem::swap(&mut store._charge, &mut ready._charge);
    std::mem::swap(&mut store.entries, &mut ready.entries);
    std::mem::swap(&mut store.queued, &mut ready.queued);
    std::mem::swap(&mut store.queued_by_army, &mut ready.queued_by_army);
    std::mem::swap(&mut store.before, &mut ready.before);
    std::mem::swap(&mut store.after, &mut ready.after);
    std::mem::swap(&mut store.raw, &mut ready.raw);
    std::mem::swap(&mut store.net, &mut ready.net);
    std::mem::swap(&mut store.armies, &mut ready.armies);
    std::mem::swap(&mut store.held, &mut ready.held);
    std::mem::swap(&mut store.reserves, &mut ready.reserves);
    std::mem::swap(&mut store.credits, &mut ready.credits);
    std::mem::swap(&mut store.queued_limit, &mut ready.queued_limit);
    CAPACITY.store(store.entries.capacity(), Ordering::Release);
    QUEUED_CAPACITY.store(store.queued.capacity(), Ordering::Release);
    DESIGNS.store(*designs, Ordering::Release);
    POOL_CAPACITY.store(store.before.len(), Ordering::Release);
    swap.retired = swap.ready.take().map(|(storage, _)| storage);
}

fn log_capacity(countries: usize, population: native::Population, designs: usize) -> Option<usize> {
    // One live-census sweep per family: deletion/volunteer/exile removals
    // each witness D divisions. Land, attrition, remainder, paradrop, merge,
    // exile, volunteer return, capitulation escape
    // and scripted division writers each get P actual division-pool entries,
    // never D times every globally saved design. Scripted stockpile scaling
    // gets S actual reserve entries. Air/garrison attribute by owner: each gets
    // C+V from the country registry and saved-design census, not their C*V
    // cross product. Their wing/occupation populations and repeated calls in
    // ANY family have no
    // per-hour bound from these censuses; excess refuses by writer name and
    // counts SITE_OVERFLOW.
    let owners_and_designs = countries.checked_add(designs)?;
    let native::Population {
        divisions,
        division_pool_entries: pool,
        stockpile_entries,
        delivery_pool_entries,
    } = population;
    [
        divisions,             // deletion witnesses
        divisions,             // volunteer transfer witnesses
        divisions,             // exile transfer witnesses
        pool,                  // land
        pool,                  // attrition
        pool,                  // deletion remainder
        pool,                  // paradrop
        pool,                  // merge main pool corrections
        delivery_pool_entries, // canceled delivery corrections
        pool,                  // exile reductions
        pool,                  // volunteer return reductions
        pool,                  // capitulation escape reductions
        pool,                  // scripted division reductions
        owners_and_designs,    // air
        owners_and_designs,    // garrison
        stockpile_entries,     // scripted stockpile reductions
    ]
    .into_iter()
    .try_fold(0usize, usize::checked_add)
}

fn event_overflow(site: SiteId) -> &'static str {
    OVERFLOW.fetch_add(1, Ordering::Relaxed);
    SITE_OVERFLOW[site as usize].fetch_add(1, Ordering::Relaxed);
    match site {
        SiteId::Land => "land damage event log overflow",
        SiteId::Attrition => "attrition event log overflow",
        SiteId::Air => "airplane loss event log overflow",
        SiteId::Garrison => "garrison resistance event log overflow",
        SiteId::DivisionFree => "division deletion event log overflow",
        SiteId::Paradrop => "paradrop disruption event log overflow",
        SiteId::Merge => "merge equipment event log overflow",
        SiteId::Return | SiteId::HqReturn => "returned equipment event log overflow",
        SiteId::VolunteerTransfer => "volunteer transfer event log overflow",
        SiteId::ExileTransfer => "exile transfer event log overflow",
        SiteId::SetExiled => "exile equipment event log overflow",
        SiteId::VolunteerReturnLoss => "volunteer return equipment event log overflow",
        SiteId::CapitulationEscapeLoss => "capitulation escape equipment event log overflow",
        SiteId::ScriptedEquipmentFraction => "scripted division equipment event log overflow",
        SiteId::ScriptedReserveEquipmentFraction => {
            "scripted stockpile equipment event log overflow"
        }
        _ => "equipment destruction event log overflow",
    }
}

/// Called before acquisition (also when no reusable frame exists). First,
/// load and discontinuity boundaries discard the previous partial interval.
pub fn boundary(clock: Option<Clock>) {
    let Some(clock) = clock else {
        lost_drain();
        return;
    };
    let Ok(mut previous) = CLOCK.try_lock() else {
        lost_drain();
        return;
    };
    let generation = crate::world_lifetime::generation();
    let old_generation = WORLD_GENERATION.swap(generation, Ordering::SeqCst);
    let world_changed = old_generation != 0 && old_generation != generation;
    let transition = world_changed
        || !previous.is_some_and(|before| {
            before.hour.checked_add(1) == Some(clock.hour)
                && before.campaign_id == clock.campaign_id
                && before.play_session_started_at == clock.play_session_started_at
        });
    let first = previous.is_none();
    let load = world_changed
        || previous.is_some_and(|before| {
            before.campaign_id != clock.campaign_id
                || before.play_session_started_at != clock.play_session_started_at
        });
    *previous = Some(clock);
    drop(previous);
    let Some(claim) = BoundaryClaim::try_enter() else {
        refuse_boundary(transition, load);
        return;
    };
    if OBSERVING.load(Ordering::SeqCst) || active() != 0 {
        refuse_boundary(transition, load);
        return;
    }
    let load = load || PENDING_LOAD.load(Ordering::SeqCst);
    let transition = transition || PENDING_BOUNDARY.load(Ordering::SeqCst);
    match STORE.try_lock() {
        Ok(mut store) => {
            PENDING_LOAD.store(false, Ordering::SeqCst);
            PENDING_BOUNDARY.store(false, Ordering::SeqCst);
            let skipped_queue = SKIPPED_QUEUE.swap(false, Ordering::SeqCst);
            store.attach_interval |= first || load || skipped_queue;
            store.boundary(transition, load);
            install_prepared(&mut store);
            if claim.finish() {
                store.transition = true;
                store.attach_interval = true;
                store.refuse(
                    None,
                    "equipment destruction boundary overlaps active observation; interval is partial",
                );
            }
        }
        Err(_) => {
            refuse_boundary(transition, load);
        }
    }
}

fn refuse_boundary(transition: bool, load: bool) {
    if transition {
        PENDING_BOUNDARY.store(true, Ordering::SeqCst);
    }
    if load {
        PENDING_LOAD.store(true, Ordering::SeqCst);
    }
    OVERLAPS.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut store) = STORE.try_lock() {
        store.transition = true;
        store.attach_interval = true;
        store.refuse(
            None,
            "equipment destruction boundary overlaps active observation; interval is partial",
        );
    } else {
        record_refusal(
            None,
            "equipment destruction boundary overlaps active observation; interval is partial",
        );
    }
}

#[cfg(test)]
pub fn drain(output: &mut [Entry]) -> Result<usize, &'static str> {
    drain_buffer(engine::walk::Buffer::Fixed(output))
}
pub fn drain_growing(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_buffer(engine::walk::Buffer::Growing(output))
}
fn drain_buffer(mut output: engine::walk::Buffer<'_, Entry>) -> Result<usize, &'static str> {
    DRAINS.fetch_add(1, Ordering::Relaxed);
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("equipment destruction hooks unavailable (see status unhooked)");
    }
    let claim = match BoundaryClaim::try_enter() {
        Some(claim) if active() == 0 && !OBSERVING.load(Ordering::SeqCst) => claim,
        _ => {
            lost_drain();
            let refusal = STORE.try_lock().ok().and_then(|store| store.refusal);
            return Err(
                refusal.map_or("equipment destruction active writer at drain", |r| r.reason)
            );
        }
    };
    let Ok(mut store) = STORE.try_lock() else {
        lost_drain();
        return Err("equipment destruction writer overlaps hourly drain");
    };
    if POISONED.load(Ordering::SeqCst)
        || store.refusal.is_some()
        || store.interval_refusal.is_some()
    {
        store.entries.clear();
        install_prepared(&mut store);
        return Err(store.refusal.or(store.interval_refusal).map_or(
            "equipment destruction lost observation; interval unavailable",
            |r| r.reason,
        ));
    }
    if store.transition {
        // This interval was already discarded. Keep the provenance window
        // open for later queued deletions, but do not refuse it a second time.
        store.unproven_queue = false;
        store.entries.clear();
        install_prepared(&mut store);
        return Err("equipment destruction first/branch interval is partial");
    }
    if store.unproven_queue {
        store.unproven_queue = false;
        store.attach_interval = false;
        store.entries.clear();
        install_prepared(&mut store);
        return Err("queued division provenance is unavailable; interval is partial");
    }
    let len = store.entries.len();
    output
        .ensure(len)
        .map_err(|_| "equipment destruction drain allocation failed")?;
    if len > output.len() {
        lost_drain();
        return Err("equipment destruction drain storage too small");
    }
    output[..len].copy_from_slice(&store.entries);
    store.entries.clear();
    install_prepared(&mut store);
    // Admission remembers even a callback that completed during the copy.
    if claim.finish() || POISONED.load(Ordering::SeqCst) || active() != 0 {
        lost_drain();
        return Err("equipment destruction writer overlaps hourly drain");
    }
    store.attach_interval = false;
    HANDED_OVER.fetch_add(len as u64, Ordering::Relaxed);
    Ok(len)
}

pub fn deletion_unavailable() -> Option<&'static str> {
    if UNHOOKED.get().is_some() {
        Some("division deletion capture requires original destruction writers")
    } else if let Some(reason) = SUPPLEMENTAL_UNHOOKED.get() {
        Some(reason.as_ref())
    } else if !SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst) {
        Some("supplemental equipment destruction hooks are disabled")
    } else {
        None
    }
}

fn boundary_running() -> bool {
    BOUNDARY_ADMISSION.load(Ordering::SeqCst) & 1 != 0
}
struct Admission;
impl Admission {
    fn enter() -> Self {
        if BOUNDARY_ADMISSION.fetch_add(4, Ordering::SeqCst) & 1 != 0 {
            BOUNDARY_ADMISSION.fetch_or(2, Ordering::SeqCst);
            poison();
        }
        Self
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        BOUNDARY_ADMISSION.fetch_sub(4, Ordering::SeqCst);
    }
}
struct BoundaryClaim(bool);
impl BoundaryClaim {
    fn try_enter() -> Option<Self> {
        let state = BOUNDARY_ADMISSION.load(Ordering::SeqCst);
        if state & !2 != 0 {
            return None;
        }
        // A stale conflict bit belongs to an earlier boundary that already
        // refused its active callback; claim once, without waiting or retry.
        BOUNDARY_ADMISSION
            .compare_exchange(state, 1, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(false))
    }
    fn finish(mut self) -> bool {
        self.0 = true;
        BOUNDARY_ADMISSION.fetch_and(!3, Ordering::SeqCst) != 1
    }
}
impl Drop for BoundaryClaim {
    fn drop(&mut self) {
        if !self.0 {
            BOUNDARY_ADMISSION.fetch_and(!3, Ordering::SeqCst);
        }
    }
}
struct Call {
    site: SiteId,
    _admission: Admission,
    unsettled: bool,
    returned: Cell<bool>,
}
impl Call {
    fn enter(site: SiteId) -> Self {
        let admission = Admission::enter();
        ACTIVE[site as usize].fetch_add(1, Ordering::SeqCst);
        CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
        if ENABLED.load(Ordering::SeqCst) {
            // Refused calls still reach their original writer. Count their
            // live burst without reading additional native operands.
            let attempted = CALL_ATTEMPTS
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            EVENT_REQUIRED.fetch_max(attempted, Ordering::Relaxed);
        }
        Self {
            site,
            _admission: admission,
            unsettled: false,
            returned: Cell::new(false),
        }
    }
}
impl Call {
    fn run<T>(&self, original: impl FnOnce() -> T) -> T {
        let result = original();
        self.returned.set(true);
        result
    }

    fn arm(&mut self) {
        self.unsettled = true;
    }
    fn settle(&mut self) {
        self.unsettled = false;
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if self.unsettled {
            observation_failed(self.site, "equipment observed native call did not settle");
        }
        // Cleanup can run while Windows still traverses our shim/native return.
        // Only normal return proves that detach may retire this callback.
        if !self.returned.get() || std::thread::panicking() {
            return;
        }
        ACTIVE[self.site as usize].fetch_sub(1, Ordering::SeqCst);
    }
}

struct Observation {
    site: SiteId,
    unsettled: bool,
}
impl Drop for Observation {
    fn drop(&mut self) {
        OBSERVING.store(false, Ordering::SeqCst);
        if self.unsettled {
            observation_failed(self.site, "equipment observed native call did not settle");
        }
    }
}

/// Own scratch without retaining a lock or a native-memory borrow across the
/// original. An overlapping pool writer runs normally and refuses its interval.
fn observe_native<T, P>(
    site: SiteId,
    pre: impl FnOnce(&mut Store) -> Result<Option<P>, &'static str>,
    original: impl FnOnce() -> T,
    post: impl FnOnce(&mut Store, P) -> Result<(), &'static str>,
) -> T {
    let generation = crate::world_lifetime::generation();
    let _admission = Admission::enter();
    if crate::world_lifetime::invalidating() {
        observation_failed(
            site,
            "equipment destruction native world awaits hourly boundary",
        );
        return original();
    }
    if boundary_running() {
        observation_failed(
            site,
            "equipment destruction observation overlaps partial boundary",
        );
        return original();
    }
    if OBSERVING.swap(true, Ordering::SeqCst) {
        observation_failed(site, "equipment destruction pool observations overlap");
        return original();
    }
    let mut observation = Observation {
        site,
        unsettled: false,
    };
    // Claim before checking Free: an earlier destructor blocks these reads,
    // while a later destructor sees OBSERVING before its native mutation.
    if ACTIVE[SiteId::DivisionFree as usize].load(Ordering::SeqCst) != 0 {
        observation_failed(
            site,
            "equipment observation overlaps active native destructor",
        );
        return original();
    }
    let before = if let Ok(mut store) = STORE.try_lock() {
        pre(&mut store)
    } else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        Err("equipment destruction pre-observation overlaps writer")
    };
    let before = match before {
        Ok(before) => before,
        Err(reason) => {
            observation_failed(site, reason);
            None
        }
    };
    observation.unsettled = before.is_some();
    let result = original();
    if let Some(before) = before {
        if generation != crate::world_lifetime::generation() {
            observation.unsettled = false;
            observation_failed(
                site,
                "equipment destruction native world changed during original",
            );
            return result;
        }
        if let Ok(mut store) = STORE.try_lock() {
            if let Err(reason) = post(&mut store, before) {
                store.refuse(Some(site as usize), reason);
            }
        } else {
            OVERLAPS.fetch_add(1, Ordering::Relaxed);
            observation_failed(
                site,
                "equipment destruction post-observation overlaps writer",
            );
        }
    }
    observation.unsettled = false;
    result
}

fn army_receiver(site: SiteId, receiver: usize) -> Result<Option<usize>, &'static str> {
    match site {
        SiteId::Land | SiteId::Paradrop | SiteId::SetExiled => Ok(Some(receiver)),
        SiteId::VolunteerReturnLoss
        | SiteId::CapitulationEscapeLoss
        | SiteId::ScriptedEquipmentFraction => Ok(Some(
            receiver
                .checked_sub(engine::carmy::EQUIPMENT_POOL)
                .ok_or("army equipment pool address underflow")?,
        )),
        _ => Ok(None),
    }
}
// Unit deletion can flush a transferred source through other native
// writers. Whole-merge scratch excludes that source's later bookkeeping too.
fn whole_merge_transfer(
    memory: &impl Memory,
    base: usize,
    army: Option<usize>,
) -> Result<bool, &'static str> {
    if !WHOLE_MERGE.get() {
        return Ok(false);
    }
    let Some(army) = army else {
        return Ok(false);
    };
    Ok(
        active_deletion_context(memory, base, army)?.is_some_and(|context| {
            context.returned || context.ignored || context.disposition != native::Disposition::Loss
        }),
    )
}

fn pool_before(
    store: &mut Store,
    site: SiteId,
    receiver: usize,
    memory: &impl Memory,
    base: usize,
) -> Result<Option<(u32, usize, usize)>, &'static str> {
    let army = army_receiver(site, receiver)?;
    let context = army
        .map(|army| active_deletion_context(memory, base, army))
        .transpose()?
        .flatten();
    if let Some(army) = army
        && transferred_source(store, memory, base, army, context)?
    {
        return Ok(None);
    }
    (|| {
        let country = if let Some(army) = army {
            army_observation_owner(memory, army, context)
        } else {
            match site {
                SiteId::Air => native::air_owner(memory, receiver),
                SiteId::Garrison => native::garrison_owner(memory, receiver),
                SiteId::ScriptedReserveEquipmentFraction => {
                    native::reserve_owner(memory, base, receiver)
                }
                _ => Err("unsupported pool destruction writer"),
            }
        }?;
        let pool = match site {
            SiteId::Land | SiteId::Paradrop | SiteId::SetExiled => receiver
                .checked_add(engine::carmy::EQUIPMENT_POOL)
                .ok_or("army equipment pool address overflow")?,
            SiteId::VolunteerReturnLoss
            | SiteId::CapitulationEscapeLoss
            | SiteId::ScriptedEquipmentFraction
            | SiteId::ScriptedReserveEquipmentFraction => receiver,
            _ => native::pool(receiver, site)?,
        };
        let len = snapshot(memory, base, pool, &mut store.before, 0)?;
        Ok(Some((country, pool, len)))
    })()
}

fn snapshot(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    rows: &mut [equipment_entry::Row],
    prefix: usize,
) -> Result<usize, &'static str> {
    native::snapshot_with_demand(memory, base, pool, rows, |count| {
        POOL_REQUIRED.fetch_max(prefix.saturating_add(count), Ordering::Relaxed);
    })
}
fn variant_amount(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
    scratch: &mut [u8],
) -> Result<i64, &'static str> {
    native::variant_amount_with_demand(memory, base, pool, variant, scratch, |count| {
        POOL_REQUIRED.fetch_max(count, Ordering::Relaxed);
    })
}
fn pool_after(
    store: &mut Store,
    site: SiteId,
    pre: Option<(u32, usize, usize)>,
    cause: native::Cause,
    memory: &impl Memory,
    base: usize,
) -> Result<(), &'static str> {
    let Some((country, pool, len)) = pre else {
        return Ok(());
    };
    let after = snapshot(memory, base, pool, &mut store.after, 0)?;
    let mut failure = None;
    let entries = &mut store.entries;
    native::removals(
        &mut store.before[..len],
        &mut store.after[..after],
        |variant, amount_raw| {
            if let Err(reason) = append_entry(
                entries,
                site,
                Entry {
                    country,
                    cause,
                    disposition: native::Disposition::Loss,
                    expeditionary_owner: 0,
                    division_handle: 0,
                    variant,
                    amount_raw,
                },
            ) {
                failure.get_or_insert(reason);
            }
        },
    )?;
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
fn observe_pool(
    store: &mut Store,
    site: SiteId,
    receiver: usize,
    cause: native::Cause,
    memory: &impl Memory,
    base: usize,
    original: impl FnOnce(),
) -> Result<(), &'static str> {
    let before = pool_before(store, site, receiver, memory, base);
    original();
    pool_after(store, site, before?, cause, memory, base)
}

fn measure_pool(site: SiteId, receiver: usize, cause: native::Cause, original: impl FnOnce()) {
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    match army_receiver(site, receiver).and_then(|army| whole_merge_transfer(&memory, base, army)) {
        Ok(true) => {
            original();
            return;
        }
        Err(reason) => {
            observation_failed(site, reason);
            original();
            return;
        }
        Ok(false) => {}
    }
    observe_native(
        site,
        |store| pool_before(store, site, receiver, &memory, base),
        original,
        |store, before| pool_after(store, site, Some(before), cause, &memory, base),
    );
}

type ArmyFn =
    unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize, usize, usize, usize);
// The entry shim copies all four stack arguments and adds the original return
// PC as argument nine. Unlike a naked framed shim, its unwind is described.
core::arch::global_asm!(
    ".text", ".globl destruction_land_entry", ".seh_proc destruction_land_entry",
    "destruction_land_entry:", "sub rsp, 0x78", ".seh_stackalloc 0x78", ".seh_endprologue",
    "mov rax, [rsp + 0xa0]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0xa8]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0xb0]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0xb8]", "mov [rsp + 0x38], rax",
    "mov rax, [rsp + 0x78]", "mov [rsp + 0x40], rax",
    "call {body}", "add rsp, 0x78", "ret", ".seh_endproc", body = sym land);
unsafe extern "system-unwind" {
    fn destruction_land_entry();
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system-unwind" fn land(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
    caller: usize,
) {
    let _call = Call::enter(SiteId::Land);
    let original: ArmyFn =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Land as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(a, b, c, d, e, f, g, h) });
    if !ENABLED.load(Ordering::SeqCst)
        || boundary_running()
        || POISONED.load(Ordering::SeqCst)
        || crate::allocation::failed()
        || b == 0
    {
        if b != 0 {
            revoke_active_observation();
            skipped_credit_mutation(Some(a));
        }
        run();
        return;
    }
    let base = BASE.load(Ordering::Relaxed);
    let cause = count_cause(
        caller
            .checked_sub(base)
            .map(native::land_cause)
            .unwrap_or(native::Cause::Unknown),
    );
    measure_pool(SiteId::Land, a, cause, run);
}

/// A tail jump with no frame: the native three arguments remain unchanged.
#[unsafe(naked)]
unsafe extern "system-unwind" fn air_entry() {
    core::arch::naked_asm!("mov r9, [rsp]", "jmp {body}", body = sym air);
}
unsafe extern "system-unwind" fn air(a: usize, b: usize, c: usize, caller: usize) {
    let _call = Call::enter(SiteId::Air);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Air as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(a, b, c) });
    if !ENABLED.load(Ordering::SeqCst)
        || boundary_running()
        || POISONED.load(Ordering::SeqCst)
        || crate::allocation::failed()
        || b as u32 == 0
    {
        if b as u32 != 0 {
            revoke_active_observation();
        }
        run();
        return;
    }
    let base = BASE.load(Ordering::Relaxed);
    let cause = count_cause(
        caller
            .checked_sub(base)
            .map(native::air_cause)
            .unwrap_or(native::Cause::Unknown),
    );
    // Airplane losses accept a negative request and add its negation;
    // observe that native gain as a signed correction too.
    measure_pool(SiteId::Air, a, cause, run);
}
unsafe extern "system-unwind" fn garrison(a: usize, b: usize, c: usize) {
    let _call = Call::enter(SiteId::Garrison);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Garrison as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(a, b, c) });
    if !ENABLED.load(Ordering::SeqCst)
        || boundary_running()
        || POISONED.load(Ordering::SeqCst)
        || crate::allocation::failed()
    {
        revoke_active_observation();
        run();
        return;
    }
    measure_pool(SiteId::Garrison, a, native::Cause::Garrison, run);
}
fn variant_before(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
) -> Result<Option<(u32, u64, i64)>, &'static str> {
    let army = pool
        .checked_sub(engine::carmy::EQUIPMENT_POOL)
        .ok_or("attrition army address underflow")?;
    let context = active_deletion_context(memory, base, army)?;
    if transferred_source(store, memory, base, army, context)? {
        return Ok(None);
    }
    let country = army_observation_owner(memory, army, context)?;
    let design = engine::cequipment_variant::identity(memory, base, variant)?;
    let before = variant_amount(memory, base, pool, variant, &mut store.raw)?;
    Ok(Some((country, design, before)))
}

fn variant_after(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
    pre: Option<(u32, u64, i64)>,
) -> Result<(), &'static str> {
    let Some((country, design, before)) = pre else {
        return Ok(());
    };
    let after = variant_amount(memory, base, pool, variant, &mut store.raw)?;
    let amount_raw = before
        .checked_sub(after)
        .ok_or("destruction difference overflow")?;
    store.append(
        SiteId::Attrition,
        Entry {
            country,
            cause: native::Cause::Attrition,
            disposition: native::Disposition::Loss,
            expeditionary_owner: 0,
            division_handle: 0,
            variant: design,
            amount_raw,
        },
    )
}

unsafe extern "system-unwind" fn attrition(pool: usize, variant: usize, quantity: usize) {
    let _call = Call::enter(SiteId::Attrition);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) = unsafe {
        std::mem::transmute(ORIGINAL[SiteId::Attrition as usize].load(Ordering::Acquire))
    };
    let run = || _call.run(|| unsafe { original(pool, variant, quantity) });
    if !ENABLED.load(Ordering::SeqCst)
        || boundary_running()
        || POISONED.load(Ordering::SeqCst)
        || crate::allocation::failed()
    {
        revoke_active_observation();
        skipped_credit_mutation(pool.checked_sub(engine::carmy::EQUIPMENT_POOL));
        run();
        return;
    }
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let army = pool.checked_sub(engine::carmy::EQUIPMENT_POOL);
    match whole_merge_transfer(&memory, base, army) {
        Ok(true) => {
            run();
            return;
        }
        Err(reason) => {
            observation_failed(SiteId::Attrition, reason);
            run();
            return;
        }
        Ok(false) => {}
    }
    observe_native(
        SiteId::Attrition,
        |store| variant_before(store, &memory, base, pool, variant),
        run,
        |store, pre| variant_after(store, &memory, base, pool, variant, Some(pre)),
    );
}

fn recording() -> bool {
    let admitted = ENABLED.load(Ordering::SeqCst)
        && !crate::world_lifetime::invalidating()
        && SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst)
        && !boundary_running()
        && !POISONED.load(Ordering::SeqCst)
        && !crate::allocation::failed();
    // Refused native originals still mutate pools used by an active observer.
    if !admitted {
        revoke_active_observation();
    }
    admitted
}

fn observation_failed(site: SiteId, reason: &'static str) {
    // Any refused native family can mutate the active observer's destinations.
    revoke_active_observation();
    if let Ok(mut store) = STORE.try_lock() {
        store.refuse(Some(site as usize), reason);
    } else {
        record_refusal(Some(site as usize), reason);
    }
}

fn transferred_source(
    store: &Store,
    memory: &impl Memory,
    base: usize,
    army: usize,
    context: Option<Deletion>,
) -> Result<bool, &'static str> {
    if context.is_some_and(|context| {
        context.ignored
            || context.returned
            || matches!(
                context.disposition,
                native::Disposition::Merged
                    | native::Disposition::VolunteerTransferred
                    | native::Disposition::ExileTransferred
            )
    }) {
        return Ok(true);
    }
    let pending = store
        .queued_by_army
        .get(&army)
        .and_then(|handle| store.queued.get(handle))
        .filter(|entry| entry.disposition != native::Disposition::Loss);
    if !store.credits.is_empty() {
        let handle = native::army_identity(memory, base, army)?;
        if store.credits.contains(handle) {
            return Ok(store.credits.state(
                handle,
                army,
                army_observation_owner(memory, army, context)?,
            )? == PoolState::Bookkeeping);
        }
    }
    // A recycled address cannot inherit another division's transfer witness.
    if let Some(pending) = pending {
        return Ok(native::army_identity(memory, base, army)? == pending.handle);
    }
    Ok(context.is_some_and(|context| context.disposition != native::Disposition::Loss))
}

fn active_deletion_context(
    memory: &impl Memory,
    base: usize,
    army: usize,
) -> Result<Option<Deletion>, &'static str> {
    let Some(context) = DELETION.get().filter(|context| context.army == army) else {
        return Ok(None);
    };
    if context.ignored && context.handle == 0 {
        return Ok(Some(context));
    }
    Ok((native::army_identity(memory, base, army)? == context.handle).then_some(context))
}

fn army_observation_owner(
    memory: &impl Memory,
    army: usize,
    context: Option<Deletion>,
) -> Result<u32, &'static str> {
    match context {
        Some(context) => Ok(context.country),
        None => native::army_owner(memory, army),
    }
}

fn take_queued(store: &mut Store, army: usize, handle: u64) -> Option<Queued> {
    if store
        .queued
        .get(&handle)
        .is_none_or(|entry| entry.army != army)
    {
        return None;
    }
    if store.queued_by_army.get(&army) == Some(&handle) {
        store.queued_by_army.remove(&army);
    }
    let entry = store.queued.remove(&handle);
    if entry.is_some() {
        store.queued_count -= 1;
    }
    entry
}

fn retire_queued(site: SiteId, army: usize, handle: Option<u64>) {
    // The original can reject a nested scheduling request or free the unit.
    // Retire its owned identity without reading that unit. Cleanup scopes
    // lacking a live handle use only the excluded object's opaque address.
    if let Ok(mut store) = STORE.try_lock() {
        if let Some(handle) = handle.or_else(|| store.queued_by_army.get(&army).copied()) {
            take_queued(&mut store, army, handle);
        }
    } else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        observation_failed(
            site,
            "equipment deletion provenance retirement overlaps writer",
        );
    }
}

fn deletion_context(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    caller: usize,
) -> Result<Option<Deletion>, &'static str> {
    let classified = caller
        .checked_sub(base)
        .map(native::deletion_context)
        .unwrap_or(native::DeletionContext::Loss(native::Cause::Unknown));
    let merge_source = classified == native::DeletionContext::Transfer(native::Disposition::Merged);
    let classified = match classified {
        native::DeletionContext::Ignore => return Ok(Some(Deletion::ignored(army))),
        native::DeletionContext::Queue => None,
        native::DeletionContext::Loss(cause) => Some((cause, native::Disposition::Loss)),
        native::DeletionContext::Transfer(disposition) => {
            Some((native::Cause::Unknown, disposition))
        }
    };
    native::current_state(memory, base, state)?;
    if native::cleanup(memory, base)? {
        // Default/partially read objects can have no live handle or owner.
        // This scope suppresses their destructor without authenticating them.
        return Ok(Some(Deletion::ignored(army)));
    }
    if !native::is_army(memory, base, army)? {
        return Ok(None);
    }
    if native::fake_intel_division(memory, army)? {
        return Ok(Some(Deletion::ignored(army)));
    }
    let handle = native::army_identity(memory, base, army)?;
    let country = native::army_owner(memory, army)?;
    let expeditionary_owner = native::army_expeditionary_owner(memory, army)?;
    let Ok(mut store) = STORE.try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        return Err("equipment deletion context overlaps writer");
    };
    let pending = take_queued(&mut store, army, handle);
    store.attach_interval |= SKIPPED_QUEUE.swap(false, Ordering::SeqCst);
    if classified.is_none() && pending.is_none() && store.attach_interval {
        store.unproven_queue = true;
        return Ok(Some(Deletion::ignored(army)));
    }
    let (cause, mut disposition) = classified.unwrap_or_else(|| {
        pending
            .map(|entry| (entry.cause, entry.disposition))
            .unwrap_or((native::Cause::Unknown, native::Disposition::Loss))
    });
    // A later direct caller cannot undo an already proved pool transfer.
    if let Some(entry) = pending.filter(|entry| entry.disposition != native::Disposition::Loss) {
        disposition = entry.disposition;
    }
    Ok(Some(Deletion {
        army,
        handle,
        country,
        cause,
        disposition,
        expeditionary_owner,
        returned: false,
        observed: false,
        ignored: false,
        merge_source,
    }))
}

/// Unit deletion keeps the entry owner before it clears expeditionary ownership.
/// Its original can flush damage through another hooked writer, so no producer
/// lock spans that call. Only the later destructor takes the residual snapshot.
#[unsafe(naked)]
unsafe extern "system-unwind" fn delete_entry() {
    core::arch::naked_asm!("mov rax, [rsp]", "jmp {body}", body = sym destruction_delete_shim);
}
core::arch::global_asm!(
    ".text", ".globl destruction_delete_shim", ".seh_proc destruction_delete_shim",
    "destruction_delete_shim:", "sub rsp, 0x38", ".seh_stackalloc 0x38", ".seh_endprologue",
    "mov [rsp + 0x20], rax", "call {body}", "add rsp, 0x38", "ret", ".seh_endproc", body = sym delete);
unsafe extern "system-unwind" {
    fn destruction_delete_shim();
}
unsafe extern "system-unwind" fn delete(
    state: usize,
    unit: usize,
    return_equipment: usize,
    behavior: usize,
    caller: usize,
) {
    let mut call = Call::enter(SiteId::Delete);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Delete as usize].load(Ordering::Acquire)) };
    let context = if recording() {
        let memory = unsafe { engine::memory::LiveMemory::new() };
        match deletion_context(&memory, BASE.load(Ordering::Relaxed), state, unit, caller) {
            Ok(context) => context,
            Err(reason) => {
                observation_failed(SiteId::Delete, reason);
                None
            }
        }
    } else {
        None
    };
    if context.is_some_and(|context| !context.ignored) {
        call.arm();
    }
    let _scope = DeletionScope::enter(context);
    call.run(|| unsafe { original(state, unit, return_equipment, behavior) });
    call.settle();
    if ENABLED.load(Ordering::SeqCst) && SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst) {
        retire_queued(
            SiteId::Delete,
            unit,
            context
                .filter(|context| context.handle != 0)
                .map(|context| context.handle),
        );
    }
}

// Bulk reserve credits saturate independently of their still-owned source
// pool. Retain actual credits until that source dies.
type ReturnedPoolContext = (usize, u64, u32, usize, usize, bool, u64);

fn return_before(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    (site, revision): (SiteId, u64),
    reserves: usize,
    pool: usize,
) -> Result<Option<ReturnedPoolContext>, &'static str> {
    let army = pool
        .checked_sub(engine::carmy::EQUIPMENT_POOL)
        .ok_or("returned army address underflow")?;
    let context = DELETION.get().filter(|context| context.army == army);
    if context.is_some_and(|context| context.ignored) {
        return Ok(None);
    }
    if site == SiteId::Return && context.is_none_or(|context| context.army != army) {
        return Err("equipment deletion returned-pool identity mismatch");
    }
    if native::cleanup(memory, base)? || native::fake_intel_division(memory, army)? {
        return Ok(None);
    }
    let handle = native::army_identity(memory, base, army)?;
    if context.is_some_and(|context| context.army != army || context.handle != handle) {
        return Err("equipment returned source handle mismatch");
    }
    let country = army_observation_owner(memory, army, context);
    #[cfg(test)]
    tests::return_revision_overlap(1, reserves, pool);
    let bookkeeping = if store.credits.contains(handle) {
        let prior = country.and_then(|country| store.credits.state(handle, army, country));
        store.credits.invalidate(handle);
        prior? == PoolState::Bookkeeping
    } else {
        store.credits.begin(handle, army, country?, revision)?;
        store.credits.invalidate(handle);
        false
    };
    let country = country?;
    let reserves = native::reserve_pool(reserves)?;
    native::reserve_owner(memory, base, reserves)?;
    let sources = snapshot(memory, base, pool, &mut store.before, 0)?;
    store.before[..sources].sort_unstable_by_key(|row| row.variant_pointer);
    let pre = native::touched_reserves(
        memory,
        base,
        reserves,
        &store.before[..sources],
        false,
        &mut store.after,
    )?;
    Ok(Some((
        army,
        handle,
        country,
        sources,
        pre,
        bookkeeping,
        revision,
    )))
}
fn return_after(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    reserves: usize,
    (army, handle, country, sources, pre, bookkeeping, revision): ReturnedPoolContext,
) -> Result<(), &'static str> {
    if native::army_identity(memory, base, army)? != handle {
        return Err("equipment returned source changed handle");
    }
    let reserves = native::reserve_pool(reserves)?;
    let (selected, post) = store.before.split_at_mut(sources);
    let post = native::touched_reserves(memory, base, reserves, selected, false, post)?;
    let mut len = 0;
    store.credits.visit(handle, army, country, |value| {
        transfers::push(&mut store.net, &mut len, value)
    })?;
    if !bookkeeping {
        // The source is copied before credit. Later excluded cleanup writes
        // to this retained bookkeeping pool cannot invent a second gain/loss.
        for row in &store.before[..sources] {
            transfers::push(
                &mut store.net,
                &mut len,
                Net {
                    country: 0,
                    variant: row.variant_handle,
                    amount: i128::from(row.amount_raw),
                    credited: 0,
                },
            )?;
        }
    }
    for row in &store.after[..pre] {
        transfers::push(
            &mut store.net,
            &mut len,
            Net {
                country: 0,
                variant: row.variant_handle,
                amount: i128::from(row.amount_raw),
                credited: -i128::from(row.amount_raw),
            },
        )?;
    }
    for row in &store.before[sources..sources + post] {
        transfers::push(
            &mut store.net,
            &mut len,
            Net {
                country: 0,
                variant: row.variant_handle,
                amount: -i128::from(row.amount_raw),
                credited: i128::from(row.amount_raw),
            },
        )?;
    }
    let len = transfers::compact(&mut store.net[..len])?;
    store.credits.check_continuity(handle)?;
    #[cfg(test)]
    tests::return_revision_overlap(
        2,
        reserves - engine::cproduction_status::STOCKPILE_POOL,
        army + engine::carmy::EQUIPMENT_POOL,
    );
    // Keep the revision observed before reads, including first-head creation.
    // A concurrent rejected callback must never supply a new certified basis.
    store.credits.begin(handle, army, country, revision)?; // keeps an explicit zero-credit witness
    store.credits.invalidate(handle);
    for value in &store.net[..len] {
        if value.amount != 0 || value.credited != 0 {
            store.credits.push(handle, *value)?;
        }
    }
    store
        .credits
        .set_state(handle, army, country, PoolState::Bookkeeping)?;
    if let Some(mut context) = DELETION
        .get()
        .filter(|context| context.army == army && context.handle == handle)
    {
        context.returned = true;
        DELETION.set(Some(context));
    }
    Ok(())
}
fn skipped_credit_mutation(army: Option<usize>) {
    if !ENABLED.load(Ordering::SeqCst) || !SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst) {
        return;
    }
    if army.is_some_and(|army| {
        DELETION
            .get()
            .is_some_and(|context| context.army == army && context.ignored)
    }) {
        return;
    }
    let source = army.and_then(|army| {
        let handle = DELETION
            .get()
            .filter(|context| context.army == army && context.handle != 0)
            .map(|context| context.handle)
            .or_else(|| {
                native::army_identity(
                    &unsafe { engine::memory::LiveMemory::new() },
                    BASE.load(Ordering::Relaxed),
                    army,
                )
                .ok()
            });
        handle.map(|handle| (army, handle))
    });
    if let Some((army, handle)) = source
        && let Ok(mut store) = STORE.try_lock()
        && store.credits.skip(handle, army)
    {
        return;
    }
    // Never wait or allocate in an overlap. A revision mismatch is a
    // persistent refusal on retained heads, including an outer observer.
    let _ = CREDIT_REVISION.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
        value.checked_add(1)
    });
}
unsafe fn returned_at(site: SiteId, reserves: usize, pool: usize) {
    let _call = Call::enter(site);
    let revision = CREDIT_REVISION.load(Ordering::SeqCst);
    let original: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[site as usize].load(Ordering::Acquire)) };
    let admitted = Cell::new(false);
    let run = || {
        if !admitted.get() {
            skipped_credit_mutation(pool.checked_sub(engine::carmy::EQUIPMENT_POOL));
        }
        _call.run(|| unsafe { original(reserves, pool) })
    };
    if !recording() {
        run();
        return;
    }
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    observe_native(
        site,
        |store| {
            let before = return_before(store, &memory, base, (site, revision), reserves, pool);
            admitted.set(before.is_ok()); // None is an authenticated native exclusion
            before
        },
        run,
        |store, pre| return_after(store, &memory, base, reserves, pre),
    );
}
unsafe extern "system-unwind" fn returned(reserves: usize, pool: usize) {
    unsafe {
        returned_at(SiteId::Return, reserves, pool);
    }
}
unsafe extern "system-unwind" fn hq_returned(reserves: usize, pool: usize) {
    unsafe {
        returned_at(SiteId::HqReturn, reserves, pool);
    }
}

/// No read follows this callback's native destructor. The marker is present
/// even for a division whose remaining pool is empty.
fn observe_deleted(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    context: Deletion,
) -> Result<(), &'static str> {
    let disposition = if context.returned && context.disposition == native::Disposition::Loss {
        native::Disposition::Returned
    } else {
        context.disposition
    };
    let cause = if disposition == native::Disposition::Loss {
        count_cause(context.cause)
    } else {
        context.cause
    };
    store.append(
        SiteId::DivisionFree,
        Entry {
            country: context.country,
            cause,
            disposition,
            expeditionary_owner: context.expeditionary_owner,
            division_handle: context.handle,
            variant: 0,
            amount_raw: 0,
        },
    )?;
    if store.credits.contains(context.handle) {
        let state = store
            .credits
            .state(context.handle, context.army, context.country);
        settle_credit(store, context)?;
        if state? == PoolState::Bookkeeping
            || context.merge_source
            || matches!(
                disposition,
                native::Disposition::Merged
                    | native::Disposition::VolunteerTransferred
                    | native::Disposition::ExileTransferred
            )
        {
            return Ok(());
        }
        // Whole Merge cleared/reallocated this pool after the old return.
        // Its old source-time deficit and new live residual are independent.
    } else {
        if matches!(
            disposition,
            native::Disposition::Returned | native::Disposition::HqTransferred
        ) {
            return Err("equipment returned source has no actual credit witness");
        }
        if disposition != native::Disposition::Loss {
            return Ok(());
        }
    }
    if OBSERVING.load(Ordering::SeqCst) {
        return Err("equipment deletion snapshot overlaps pool observation");
    }
    let pool = context
        .army
        .checked_add(engine::carmy::EQUIPMENT_POOL)
        .ok_or("deleted army equipment pool address overflow")?;
    let mut len = 0;
    let rows = snapshot(memory, base, pool, &mut store.before, 0)?;
    for row in &store.before[..rows] {
        transfers::push(
            &mut store.net,
            &mut len,
            Net {
                country: 0,
                variant: row.variant_handle,
                amount: i128::from(row.amount_raw),
                credited: 0,
            },
        )?;
    }
    let len = transfers::compact(&mut store.net[..len])?;
    for index in 0..len {
        let value = store.net[index];
        let amount_raw = i64::try_from(value.amount)
            .map_err(|_| "equipment deletion correction exceeds signed width")?;
        store.append(
            SiteId::DivisionFree,
            Entry {
                country: context.country,
                cause,
                disposition: native::Disposition::Loss,
                expeditionary_owner: 0,
                division_handle: 0,
                variant: value.variant,
                amount_raw,
            },
        )?;
    }
    Ok(())
}

#[unsafe(naked)]
unsafe extern "system-unwind" fn division_free_entry() {
    core::arch::naked_asm!("jmp {body}", body = sym division_free);
}
unsafe extern "system-unwind" fn division_free(army: usize, flags: usize) -> usize {
    let mut call = Call::enter(SiteId::DivisionFree);
    let original: unsafe extern "system-unwind" fn(usize, usize) -> usize = unsafe {
        std::mem::transmute(ORIGINAL[SiteId::DivisionFree as usize].load(Ordering::Acquire))
    };
    let mut direct_scope = None;
    if recording() {
        if DELETION.get().is_none_or(|context| context.army != army) {
            let memory = unsafe { engine::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            let context = (|| {
                if native::cleanup(&memory, base)? || native::fake_intel_division(&memory, army)? {
                    return Ok(None);
                }
                let handle = native::army_identity(&memory, base, army)?;
                let Ok(mut store) = STORE.try_lock() else {
                    OVERLAPS.fetch_add(1, Ordering::Relaxed);
                    return Err("equipment deletion direct context overlaps writer");
                };
                let pending = take_queued(&mut store, army, handle);
                Ok(Some(Deletion {
                    army,
                    handle,
                    country: native::army_owner(&memory, army)?,
                    cause: pending.map_or(native::Cause::Unknown, |entry| entry.cause),
                    disposition: pending
                        .map_or(native::Disposition::Loss, |entry| entry.disposition),
                    expeditionary_owner: native::army_expeditionary_owner(&memory, army)?,
                    returned: false,
                    observed: false,
                    ignored: false,
                    merge_source: false,
                }))
            })();
            match context {
                Ok(Some(context)) => {
                    direct_scope = Some(DeletionScope::enter(Some(context)));
                }
                Ok(None) => {}
                Err(reason) => observation_failed(SiteId::DivisionFree, reason),
            }
        }
        if let Some(mut context) = DELETION.get()
            && context.army == army
            && !context.ignored
            && !context.observed
        {
            context.observed = true;
            DELETION.set(Some(context));
            let result = if let Ok(mut store) = STORE.try_lock() {
                let memory = unsafe { engine::memory::LiveMemory::new() };
                observe_deleted(&mut store, &memory, BASE.load(Ordering::Relaxed), context)
            } else {
                OVERLAPS.fetch_add(1, Ordering::Relaxed);
                Err("equipment deletion snapshot overlaps writer")
            };
            if let Err(reason) = result {
                observation_failed(SiteId::DivisionFree, reason);
            }
        }
    }
    let enabled = ENABLED.load(Ordering::SeqCst)
        && SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst)
        && !DELETION
            .get()
            .is_some_and(|context| context.army == army && context.ignored);
    let retired = DELETION
        .get()
        .filter(|context| context.army == army && context.handle != 0)
        .map(|context| context.handle)
        .or_else(|| {
            enabled
                .then(|| {
                    native::army_identity(
                        &unsafe { engine::memory::LiveMemory::new() },
                        BASE.load(Ordering::Relaxed),
                        army,
                    )
                    .ok()
                })
                .flatten()
        });
    if enabled {
        if let Some(handle) = retired {
            if let Ok(mut store) = STORE.try_lock() {
                store.credits.invalidate(handle);
            } else {
                skipped_credit_mutation(None);
                observation_failed(
                    SiteId::DivisionFree,
                    "equipment deletion proof overlaps writer",
                );
            }
        } else {
            skipped_credit_mutation(Some(army));
        }
    }
    let context = DELETION.get().filter(|context| context.army == army);
    let merge_source = WHOLE_MERGE.get()
        && context.is_some_and(|context| {
            context.merge_source
                && native::army_identity(
                    &unsafe { engine::memory::LiveMemory::new() },
                    BASE.load(Ordering::Relaxed),
                    army,
                ) == Ok(context.handle)
        });
    if OBSERVING.load(Ordering::SeqCst)
        && !merge_source
        && context.is_none_or(|context| !context.ignored)
    {
        // Its destructor returns held deliveries,
        // including shared reserves outside the authenticated Merge inputs.
        observation_failed(
            SiteId::DivisionFree,
            "equipment deletion native pools overlap observation",
        );
    }
    if DELETION.get().is_some_and(|context| !context.ignored) {
        call.arm();
    }
    let result = call.run(|| unsafe { original(army, flags) });
    call.settle();
    if enabled {
        retire_queued(SiteId::DivisionFree, army, retired);
        if let Some(handle) = retired {
            if let Ok(mut store) = STORE.try_lock() {
                store.credits.remove(handle);
            } else {
                observation_failed(
                    SiteId::DivisionFree,
                    "equipment deletion proof retirement overlaps writer",
                );
            }
        }
    }
    drop(direct_scope);
    result
}

fn record_queue(
    store: &mut Store,
    army: usize,
    handle: u64,
    cause: native::Cause,
    disposition: native::Disposition,
) -> Result<(), &'static str> {
    if let Some(previous) = store.queued_by_army.get(&army).copied()
        && previous != handle
    {
        take_queued(store, army, previous);
    }
    if let Some(entry) = store.queued.get_mut(&handle) {
        if entry.army != army {
            return Err("equipment deletion handle has conflicting army address");
        }
        if entry.cause != cause {
            entry.cause = native::Cause::Unknown;
        }
        if disposition != native::Disposition::Loss {
            entry.disposition = disposition;
        }
        return Ok(());
    }
    if store.queued_count == store.queued_limit {
        OVERFLOW.fetch_add(1, Ordering::Relaxed);
        SITE_OVERFLOW[SiteId::Queue as usize].fetch_add(1, Ordering::Relaxed);
        return Err("equipment deletion provenance log overflow");
    }
    store.queued.insert(
        handle,
        Queued {
            army,
            handle,
            cause,
            disposition,
        },
    )?;
    if let Err(reason) = store.queued_by_army.insert(army, handle) {
        store.queued.remove(&handle);
        return Err(reason);
    }
    store.queued_count += 1;
    Ok(())
}

#[unsafe(naked)]
unsafe extern "system-unwind" fn queue_entry() {
    core::arch::naked_asm!("mov r8, [rsp]", "jmp {body}", body = sym queue);
}
unsafe extern "system-unwind" fn queue(state: usize, unit: usize, caller: usize) {
    let mut call = Call::enter(SiteId::Queue);
    let original: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Queue as usize].load(Ordering::Acquire)) };
    if recording() {
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let result = (|| {
            let classified = caller
                .checked_sub(base)
                .map(native::queued_context)
                .unwrap_or(native::QueuedContext::Loss(native::Cause::Unknown));
            let (cause, disposition) = match classified {
                native::QueuedContext::Ignore => return Ok(false),
                native::QueuedContext::Loss(cause) => (cause, native::Disposition::Loss),
                native::QueuedContext::Transfer(disposition) => {
                    (native::Cause::Unknown, disposition)
                }
            };
            native::current_state(&memory, base, state)?;
            if native::cleanup(&memory, base)?
                || !native::is_army(&memory, base, unit)?
                || native::fake_intel_division(&memory, unit)?
            {
                return Ok(false);
            }
            let handle = native::army_identity(&memory, base, unit)?;
            let Ok(mut store) = STORE.try_lock() else {
                OVERLAPS.fetch_add(1, Ordering::Relaxed);
                return Err("equipment deletion provenance overlaps writer");
            };
            record_queue(&mut store, unit, handle, cause, disposition).map(|()| true)
        })();
        if result == Ok(true) {
            call.arm();
        }
        if let Err(reason) = result {
            observation_failed(SiteId::Queue, reason);
        }
    } else {
        // Poison, enable ordering and boundary refusal can skip a real native
        // queue request. Its later unit deletion must not fabricate Unknown loss.
        SKIPPED_QUEUE.store(true, Ordering::SeqCst);
    }
    call.run(|| unsafe { original(state, unit) });
    call.settle();
}

unsafe extern "system-unwind" fn paradrop(
    army: usize,
    source: usize,
    equipment: usize,
    manpower: usize,
    organisation: usize,
) {
    let _call = Call::enter(SiteId::Paradrop);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Paradrop as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(army, source, equipment, manpower, organisation) });
    if !recording() {
        skipped_credit_mutation(Some(army));
        run();
        return;
    }
    measure_pool(
        SiteId::Paradrop,
        army,
        native::Cause::ParadropDisruption,
        run,
    );
}

fn transfer_before(
    memory: &impl Memory,
    base: usize,
    army: usize,
    disposition: native::Disposition,
) -> Result<Option<Entry>, &'static str> {
    if native::cleanup(memory, base)? || native::fake_intel_division(memory, army)? {
        return Ok(None);
    }
    Ok(Some(Entry {
        country: native::army_owner(memory, army)?,
        expeditionary_owner: native::army_expeditionary_owner(memory, army)?,
        division_handle: native::army_identity(memory, base, army)?,
        disposition,
        ..Default::default()
    }))
}

unsafe fn transfer_roster(
    site: SiteId,
    country: usize,
    army: usize,
    disposition: native::Disposition,
) -> usize {
    let mut call = Call::enter(site);
    let original: unsafe extern "system-unwind" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[site as usize].load(Ordering::Acquire)) };
    let pending = if recording() {
        let memory = unsafe { engine::memory::LiveMemory::new() };
        transfer_before(&memory, BASE.load(Ordering::Relaxed), army, disposition)
    } else {
        Ok(None)
    };
    if pending.as_ref().is_ok_and(|entry| entry.is_some()) {
        call.arm();
    }
    if let Err(reason) = pending {
        observation_failed(site, reason);
    }
    let result = call.run(|| unsafe { original(country, army) });
    match pending {
        Ok(Some(entry)) if result as u8 != 0 => {
            if let Ok(mut store) = STORE.try_lock() {
                if let Err(reason) = store.append(site, entry) {
                    store.refuse(Some(site as usize), reason);
                }
            } else {
                OVERLAPS.fetch_add(1, Ordering::Relaxed);
                observation_failed(site, "division roster transfer overlaps writer");
            }
        }
        _ => {}
    }
    call.settle();
    result
}
unsafe extern "system-unwind" fn volunteer_transfer(country: usize, army: usize) -> usize {
    unsafe {
        transfer_roster(
            SiteId::VolunteerTransfer,
            country,
            army,
            native::Disposition::VolunteerTransferred,
        )
    }
}
unsafe extern "system-unwind" fn exile_transfer(country: usize, army: usize) -> usize {
    unsafe {
        transfer_roster(
            SiteId::ExileTransfer,
            country,
            army,
            native::Disposition::ExileTransferred,
        )
    }
}

unsafe fn pool_scale(site: SiteId, receiver: usize, factor: usize, cause: native::Cause) {
    let _call = Call::enter(site);
    let original: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[site as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(receiver, factor) });
    if recording() {
        measure_pool(site, receiver, cause, run);
    } else {
        if let Ok(Some(army)) = army_receiver(site, receiver) {
            skipped_credit_mutation(Some(army));
        }
        run();
    }
}
unsafe extern "system-unwind" fn set_exiled(army: usize, exiled: usize) {
    unsafe {
        pool_scale(
            SiteId::SetExiled,
            army,
            exiled,
            native::Cause::ExileEquipment,
        );
    }
}
unsafe extern "system-unwind" fn volunteer_return_loss(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::VolunteerReturnLoss,
            pool,
            factor,
            native::Cause::VolunteerReturnEquipment,
        );
    }
}
unsafe extern "system-unwind" fn capitulation_escape_loss(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::CapitulationEscapeLoss,
            pool,
            factor,
            native::Cause::CapitulationEscapeEquipment,
        );
    }
}
unsafe extern "system-unwind" fn scripted_equipment_fraction(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::ScriptedEquipmentFraction,
            pool,
            factor,
            native::Cause::ScriptedEquipmentFraction,
        );
    }
}
unsafe extern "system-unwind" fn scripted_reserve_equipment_fraction(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::ScriptedReserveEquipmentFraction,
            pool,
            factor,
            native::Cause::ScriptedStockpileFraction,
        );
    }
}

struct WholeMergeScope(bool);
impl WholeMergeScope {
    fn enter() -> Self {
        Self(WHOLE_MERGE.replace(true))
    }
}
impl Drop for WholeMergeScope {
    fn drop(&mut self) {
        WHOLE_MERGE.set(self.0);
    }
}

// Merging copies signed nonzero source/survivor entries, clears the
// survivor, saturates each retained allocation, and can round reserve credits.
// Observe physical pools once; source pools are gone after the native call.
struct MergeBefore {
    country: u32,
    survivor: usize,
    handle: u64,
    main_reserve: usize,
    sources: usize,
    scopes: usize,
    len: usize,
    prior_credit: bool,
}
fn merge_before(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    argument: usize,
) -> Result<Option<MergeBefore>, &'static str> {
    if native::cleanup(memory, base)? {
        return Ok(None);
    }
    let (survivor, armies) = native::merge_armies(memory, base, argument, &mut store.armies)?;
    let handle = native::army_identity(memory, base, survivor)?;
    let country = native::merge_owner(memory, survivor);
    let prior_credit = store.credits.contains(handle);
    let prior_state = if prior_credit {
        let state = country.and_then(|country| store.credits.state(handle, survivor, country));
        store.credits.invalidate(handle);
        Some(state?)
    } else {
        None
    };
    let country = country?;
    // Selected main quantities transfer to the survivor's aggregate before
    // allocation and credit, both survivor-owned. Its
    // net correction therefore belongs to that recipient, even for mixed owners.
    // Canceled delivery returns instead belong to each resolved reserve owner.
    let mut sources = 0;
    let mut len = 0;
    for &army in &store.armies[..armies] {
        let pool = army
            .checked_add(engine::carmy::EQUIPMENT_POOL)
            .ok_or("merge pool address overflow")?;
        let rows = snapshot(memory, base, pool, &mut store.before[sources..], sources)?;
        let handle = native::army_identity(memory, base, army)?;
        let credited = if army == survivor {
            prior_state == Some(PoolState::Bookkeeping)
        } else {
            store.credits.contains(handle)
                && store
                    .credits
                    .state(handle, army, native::army_owner(memory, army)?)?
                    == PoolState::Bookkeeping
        };
        // Returned bookkeeping still selects credits. Its source-time deficit
        // settles separately; live donor inventory remains a real input.
        if !credited {
            for row in &store.before[sources..sources + rows] {
                transfers::push(
                    &mut store.net,
                    &mut len,
                    Net {
                        country,
                        variant: row.variant_handle,
                        amount: i128::from(row.amount_raw),
                        credited: 0,
                    },
                )?;
            }
        }
        sources += rows;
    }
    store.before[..sources].sort_unstable_by_key(|row| row.variant_pointer);
    let main_reserve = if store.before[..sources].iter().any(|row| row.amount_raw > 0) {
        native::merge_reserves(memory, base, survivor)?
    } else {
        0
    };
    let mut held = 0;
    for &army in &store.armies[..armies] {
        let mut destination = None;
        // Destructed source families resolve their own roster country. Only
        // reinforcement is canceled on the survivor.
        native::canceled_delivery_pools(memory, base, army, army != survivor, |pool| {
            let rows = snapshot(memory, base, pool, &mut store.after, 0)?;
            if rows == 0 {
                return Ok(());
            }
            let (pool, identity) = if let Some(destination) = destination {
                destination
            } else {
                let pool = native::merge_reserves(memory, base, army)?;
                let identity = native::reserve_owner(memory, base, pool)?;
                destination = Some((pool, identity));
                (pool, identity)
            };
            for &row in &store.after[..rows] {
                *store
                    .held
                    .get_mut(held)
                    .ok_or("merge held delivery scratch too short")? = HeldRow {
                    destination: pool,
                    country: identity,
                    row,
                };
                held += 1;
            }
            Ok(())
        })?;
    }
    store.held[..held].sort_unstable_by_key(|held| (held.destination, held.row.variant_pointer));
    let mut scopes = 0;
    let mut start = 0;
    while start < held {
        let first = store.held[start];
        let mut end = start + 1;
        while end < held && store.held[end].destination == first.destination {
            if store.held[end].country != first.country {
                return Err("merge reserve destination changed identity");
            }
            end += 1;
        }
        *store
            .reserves
            .get_mut(scopes)
            .ok_or("merge reserve scopes scratch too short")? = MergeReserve {
            pool: first.destination,
            country: if first.destination == main_reserve {
                country
            } else {
                first.country
            },
            identity: first.country,
            held_start: start,
            held_end: end,
        };
        scopes += 1;
        start = end;
    }
    if main_reserve != 0
        && !store.reserves[..scopes]
            .iter()
            .any(|scope| scope.pool == main_reserve)
    {
        *store
            .reserves
            .get_mut(scopes)
            .ok_or("merge reserve scopes scratch too short")? = MergeReserve {
            pool: main_reserve,
            country,
            identity: native::reserve_owner(memory, base, main_reserve)?,
            held_start: 0,
            held_end: 0,
        };
        scopes += 1;
    }
    for scope in &store.reserves[..scopes] {
        for held in &store.held[scope.held_start..scope.held_end] {
            transfers::push(
                &mut store.net,
                &mut len,
                Net {
                    country: scope.country,
                    variant: held.row.variant_handle,
                    amount: i128::from(held.row.amount_raw),
                    credited: 0,
                },
            )?;
        }
        let rows = native::touched_reserves_if(
            memory,
            base,
            scope.pool,
            |variant| {
                merge_selected(
                    &store.before[..sources],
                    &store.held[scope.held_start..scope.held_end],
                    scope.pool == main_reserve,
                    variant,
                )
            },
            &mut store.after,
        )?;
        for row in &store.after[..rows] {
            transfers::push(
                &mut store.net,
                &mut len,
                Net {
                    country: scope.country,
                    variant: row.variant_handle,
                    amount: i128::from(row.amount_raw),
                    credited: 0,
                },
            )?;
        }
    }
    Ok(Some(MergeBefore {
        country,
        survivor,
        handle,
        main_reserve,
        sources,
        scopes,
        len,
        prior_credit,
    }))
}
fn merge_selected(
    sources: &[equipment_entry::Row],
    held: &[HeldRow],
    main: bool,
    variant: usize,
) -> bool {
    let start = sources.partition_point(|row| row.variant_pointer < variant);
    (main
        && sources[start..]
            .iter()
            .take_while(|row| row.variant_pointer == variant)
            .any(|row| row.amount_raw > 0))
        || held
            .binary_search_by_key(&variant, |held| held.row.variant_pointer)
            .is_ok()
}
fn merge_after(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    MergeBefore {
        country,
        survivor,
        handle,
        main_reserve,
        sources,
        scopes,
        mut len,
        prior_credit,
    }: MergeBefore,
) -> Result<(), &'static str> {
    if native::army_identity(memory, base, survivor)? != handle {
        return Err("merge survivor changed handle");
    }
    let pool = survivor
        .checked_add(engine::carmy::EQUIPMENT_POOL)
        .ok_or("merge pool address overflow")?;
    let rows = snapshot(memory, base, pool, &mut store.after, 0)?;
    for row in &store.after[..rows] {
        transfers::push(
            &mut store.net,
            &mut len,
            Net {
                country,
                variant: row.variant_handle,
                amount: -i128::from(row.amount_raw),
                credited: 0,
            },
        )?;
    }
    for scope in &store.reserves[..scopes] {
        if native::reserve_owner(memory, base, scope.pool)? != scope.identity {
            return Err("merge reserve destination changed identity");
        }
        let rows = native::touched_reserves_if(
            memory,
            base,
            scope.pool,
            |variant| {
                merge_selected(
                    &store.before[..sources],
                    &store.held[scope.held_start..scope.held_end],
                    scope.pool == main_reserve,
                    variant,
                )
            },
            &mut store.after,
        )?;
        for row in &store.after[..rows] {
            transfers::push(
                &mut store.net,
                &mut len,
                Net {
                    country: scope.country,
                    variant: row.variant_handle,
                    amount: -i128::from(row.amount_raw),
                    credited: 0,
                },
            )?;
        }
    }
    let len = transfers::compact(&mut store.net[..len])?;
    for index in 0..len {
        let value = store.net[index];
        let amount_raw = i64::try_from(value.amount)
            .map_err(|_| "merge equipment correction exceeds signed width")?;
        store.append(
            SiteId::Merge,
            Entry {
                country: value.country,
                cause: native::Cause::DivisionMerged,
                disposition: native::Disposition::Loss,
                expeditionary_owner: 0,
                division_handle: 0,
                variant: value.variant,
                amount_raw,
            },
        )?;
    }
    if prior_credit {
        store
            .credits
            .set_state(handle, survivor, country, PoolState::Live)?;
    }
    Ok(())
}
unsafe extern "system-unwind" fn merge(argument: usize) {
    let _call = Call::enter(SiteId::Merge);
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Merge as usize].load(Ordering::Acquire)) };
    let admitted = Cell::new(false);
    let run = || {
        // A skipped whole operation can mutate survivor and donor lifetimes;
        // its failed admission has no authenticated complete receiver set.
        if !admitted.get() {
            skipped_credit_mutation(None);
        }
        _call.run(|| unsafe { original(argument) })
    };
    if !recording() {
        run();
        return;
    }
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let _scope = WholeMergeScope::enter();
    observe_native(
        SiteId::Merge,
        |store| {
            let before = merge_before(store, &memory, base, argument);
            admitted.set(before.is_ok());
            before
        },
        run,
        |store, pre| merge_after(store, &memory, base, pre),
    );
}

fn verify_sites(
    memory: &impl Memory,
    base: usize,
    supplemental: bool,
) -> Result<(), Cow<'static, str>> {
    for site in native::SITES
        .into_iter()
        .filter(|site| site.id.is_supplemental() == supplemental)
    {
        if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
            use std::fmt::Write;
            let mut reason = String::new();
            engine::allocation::reserve_string(&mut reason, site.name.len() + 96)
                .map_err(|_| Cow::Borrowed("collector allocation failed"))?;
            write!(
                reason,
                "{} sentinel differs or unreadable at {:#x}",
                site.name,
                base + site.rva
            )
            .expect("writing a reserved String cannot fail");
            return Err(Cow::Owned(reason));
        }
    }
    Ok(())
}

fn callback(id: native::SiteId) -> usize {
    use native::SiteId;
    match id {
        SiteId::Land => destruction_land_entry as *const () as usize,
        SiteId::Attrition => attrition as *const () as usize,
        SiteId::Air => air_entry as *const () as usize,
        SiteId::Garrison => garrison as *const () as usize,
        SiteId::Delete => delete_entry as *const () as usize,
        SiteId::Return => returned as *const () as usize,
        SiteId::HqReturn => hq_returned as *const () as usize,
        SiteId::Merge => merge as *const () as usize,
        SiteId::DivisionFree => division_free_entry as *const () as usize,
        SiteId::Queue => queue_entry as *const () as usize,
        SiteId::Paradrop => paradrop as *const () as usize,
        SiteId::VolunteerTransfer => volunteer_transfer as *const () as usize,
        SiteId::ExileTransfer => exile_transfer as *const () as usize,
        SiteId::SetExiled => set_exiled as *const () as usize,
        SiteId::VolunteerReturnLoss => volunteer_return_loss as *const () as usize,
        SiteId::CapitulationEscapeLoss => capitulation_escape_loss as *const () as usize,
        SiteId::ScriptedEquipmentFraction => scripted_equipment_fraction as *const () as usize,
        SiteId::ScriptedReserveEquipmentFraction => {
            scripted_reserve_equipment_fraction as *const () as usize
        }
    }
}

fn set_sites_enabled(hooks: &mut [(native::SiteId, Hook)], enabled: bool) -> Result<bool, String> {
    let mut family = crate::hook::publication_storage(hooks.len(), enabled)
        .map_err(|_| "equipment hook family allocation failed".to_owned())?;
    family.extend(hooks.iter_mut().map(|(_, hook)| hook));
    let complete = Hook::set_enabled_batch(&mut family, enabled)?;
    drop(family);
    if complete {
        for (id, _) in hooks {
            SITE_ENABLED[*id as usize].store(enabled, Ordering::SeqCst);
        }
    }
    Ok(complete)
}

pub struct Hooks {
    original: Vec<(native::SiteId, Hook)>,
    publication_attempted: bool,
    supplemental: Vec<(native::SiteId, Hook)>,
}
impl Hooks {
    /// The original writers and optional supplemental group verify
    /// and install independently. No member of a refused group is enabled.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        // Initialize the store on the loader before any writer can enter.
        LazyLock::force(&STORE);
        BASE.store(base, Ordering::Relaxed);
        let install = |supplemental| -> Result<Vec<(native::SiteId, Hook)>, Cow<'static, str>> {
            verify_sites(memory, base, supplemental)?;
            let count = native::SITES
                .iter()
                .filter(|site| site.id.is_supplemental() == supplemental)
                .count();
            let mut hooks = engine::allocation::vec(count)
                .map_err(|_| Cow::Borrowed("collector allocation failed"))?;
            for site in native::SITES
                .into_iter()
                .filter(|site| site.id.is_supplemental() == supplemental)
            {
                let hook = match site.kind {
                    native::SiteKind::Entry => unsafe {
                        Hook::new(base + site.rva, callback(site.id), site.sentinel)
                    },
                    native::SiteKind::Call { .. } => unsafe {
                        Hook::new_call(base + site.rva, callback(site.id), site.sentinel)
                    },
                }
                .map_err(Cow::Owned)?;
                ORIGINAL[site.id as usize].store(
                    match site.kind {
                        native::SiteKind::Entry => hook.trampoline(),
                        native::SiteKind::Call { target } => base + target,
                    },
                    Ordering::Release,
                );
                hooks.push((site.id, hook));
            }
            for &(id, _) in &hooks {
                INSTALLED[id as usize].store(true, Ordering::Release);
            }
            Ok(hooks)
        };
        let original = install(false).unwrap_or_else(|reason| {
            let _ = UNHOOKED.set(reason);
            Vec::new()
        });
        let supplemental = if original.is_empty() {
            let _ = SUPPLEMENTAL_UNHOOKED
                .set("supplemental skipped: original writers unavailable".into());
            Vec::new()
        } else {
            install(true).unwrap_or_else(|reason| {
                let _ = SUPPLEMENTAL_UNHOOKED.set(reason);
                Vec::new()
            })
        };
        Self {
            original,
            supplemental,
            publication_attempted: false,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        self.publication_attempted |= enabled;
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
            let original = set_sites_enabled(&mut self.original, false);
            let supplemental = set_sites_enabled(&mut self.supplemental, false);
            return match (original, supplemental) {
                (Err(reason), _) | (_, Err(reason)) => Err(reason),
                (Ok(a), Ok(b)) => Ok(a && b),
            };
        }
        if !set_sites_enabled(&mut self.original, enabled)? {
            return Ok(false);
        }
        let original_count = native::SITES
            .iter()
            .filter(|site| !site.id.is_supplemental())
            .count();
        ENABLED.store(
            enabled && self.original.len() == original_count,
            Ordering::SeqCst,
        );
        let supplemental_enabled =
            ENABLED.load(Ordering::SeqCst) && SUPPLEMENTAL_UNHOOKED.get().is_none();
        match set_sites_enabled(&mut self.supplemental, supplemental_enabled) {
            Ok(false) => return Ok(false),
            Err(reason) if supplemental_enabled => {
                SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
                let _ = SUPPLEMENTAL_UNHOOKED.set(Cow::Owned(reason));
                // Preserve hook storage until detach. A safe rollback is
                // necessary before the original group can run independently.
                if !set_sites_enabled(&mut self.supplemental, false)? {
                    return Ok(false);
                }
            }
            Err(reason) => return Err(reason),
            Ok(true) => {}
        }
        let supplemental_count = native::SITES
            .iter()
            .filter(|site| site.id.is_supplemental())
            .count();
        SUPPLEMENTAL_ENABLED.store(
            ENABLED.load(Ordering::SeqCst)
                && SUPPLEMENTAL_UNHOOKED.get().is_none()
                && self.supplemental.len() == supplemental_count,
            Ordering::SeqCst,
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.original
            .iter()
            .chain(&self.supplemental)
            .flat_map(|(_, hook)| hook.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        if !self.publication_attempted {
            let _ = release();
        }
    }
}
pub fn status() -> impl serde::Serialize {
    let refusal = FIRST_REFUSAL.get().copied();
    let hooks = std::array::from_fn::<_, { native::SITES.len() }, _>(|i| {
        let site = &native::SITES[i];
        crate::status::status!({
            "site_index": i, "name": site.name, "rva": site.rva, "group": if site.id.is_supplemental() { "supplemental" } else { "original_writers" }, "sentinel": site.sentinel,
            "installed": INSTALLED[i].load(Ordering::Acquire),
            "enabled": SITE_ENABLED[i].load(Ordering::SeqCst),
            "active": ACTIVE[i].load(Ordering::SeqCst),
            "calls": CALLS[i].load(Ordering::Relaxed),
            "refusals": SITE_REFUSALS[i].load(Ordering::Relaxed),
            "overflow": SITE_OVERFLOW[i].load(Ordering::Relaxed),
        })
    });
    crate::status::status!({"hooks":crate::status::sequence(hooks.into_iter()),"unhooked":UNHOOKED.get(),"supplemental_unhooked":SUPPLEMENTAL_UNHOOKED.get(),"supplemental_enabled":SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst),"branch_unavailable":POISONED.load(Ordering::SeqCst),
        "events":EVENTS.load(Ordering::Relaxed),"overflow":OVERFLOW.load(Ordering::Relaxed),"refusals":REFUSALS.load(Ordering::Relaxed),"refusal":refusal,
        "lost_drains":LOST.load(Ordering::Relaxed),"overlaps":OVERLAPS.load(Ordering::Relaxed),"drains":DRAINS.load(Ordering::Relaxed),
        "handed_over":HANDED_OVER.load(Ordering::Relaxed),"event_capacity":capacity(),"queue_capacity":QUEUED_CAPACITY.load(Ordering::Acquire),"design_capacity":DESIGNS.load(Ordering::Acquire),
        "prepare_failures":PREPARE_FAILURES.load(Ordering::Relaxed),
        "unknown_callers":UNKNOWN_CALLERS.load(Ordering::Relaxed),
        "hourly_peak_events":HOURLY_PEAK_EVENTS.load(Ordering::Relaxed),"hourly_peak_capacity":HOURLY_PEAK_CAPACITY.load(Ordering::Relaxed),
        "scope":"cumulative since attach; stable causes; unknown callers counted; peak includes partial or refused intervals; refusals recover at clean boundaries; first and branch intervals are partial"})
}

#[cfg(test)]
mod tests;
