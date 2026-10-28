//! The retained regional air-combat records AirRegionCombat records, read
//! only where the native record updater wrote them
//! ([`engine_air_history::air_record_mutations`]).
//!
//! The air-combat record updater is the only run-time
//! writer of a holder's records. Its detour notes the holder (RCX) in a
//! lock-free set and runs the original under a writer lease; it never
//! allocates, waits or reads game memory. The capture, on the FinalInputs
//! worker while the game thread is parked, closes the set's gate, takes the
//! holders it names and reopens it: a writer at the closed gate, a lease
//! spanning it, a full set, a world reset in progress or a disabled family
//! loses the interval, and the hour reads every holder.
//!
//! The store holds every holder of the latest complete capture: its key (container country and
//! region), address and the records it held. An hour whose interval was observed whole reads only
//! the touched holders, and of each only the records dated at or after the previous capture's hour
//! (the updater writes no other: its callers run after the clock advance, hourly update), splicing
//! the older ones from the held row by the native counts: the daily prune removes only a leading
//! run, so the native records before the tail are the held ones less that run. A touched holder the
//! store does not hold is new: it is located through its owner's Table  (air-combat holder
//! construction copied the owner from the strategic air object) and read whole. Every other holder
//! is carried: its row is its native state at its last read, which playback prunes
//! ([`replay_derive::air_region_derive`]).
//!
//! A touched holder that cannot be spliced, or that its owner's table does
//! not name, poisons the family: every later hour reads every holder.
use crate::hook::Hook;
use engine_air_history::{air_record_mutations::MUTATIONS, cair_region_combat_data as native};
use engine_native::Memory;
use replay_data::{
    rows::AirCombatDamage, rows::AirCombatRecord, rows::AirRegionCombatRow, rows::TableRows,
};
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static TOUCHED: Touched = Touched::new();
static STORE: Mutex<Option<Store>> = Mutex::new(None);
static POISONED: AtomicBool = AtomicBool::new(false);
/// Hours that read every holder, by cause: no held capture, an invalidating
/// hour, a lost interval, a poisoned family, a difference found while
/// carrying.
static FULL: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Carried hours, holders read for a touch, records those reads took,
/// holders found new, holders carried.
static COUNTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// Smallest touched-holder set: a set grows from live demand, never caps it.
const MIN_SLOTS: usize = 1024;

/// One interval's touched holder addresses: open addressing, each slot
/// leaving vacancy at most once until an exclusive take, so racing writers
/// settle on one slot by compare-exchange alone. `claims` lists the taken
/// slots so a take visits only those.
struct Set {
    slots: Vec<AtomicUsize>,
    claims: Vec<AtomicUsize>,
    claimed: AtomicUsize,
}

impl Set {
    /// Off the game thread: room for `holders`, at most half full.
    fn new(holders: usize) -> Option<Box<Self>> {
        let len = holders
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)?
            .max(MIN_SLOTS);
        let mut slots = engine_allocation::vec(len).ok()?;
        slots.resize_with(len, AtomicUsize::default);
        let mut claims = engine_allocation::vec(len).ok()?;
        claims.resize_with(len, AtomicUsize::default);
        engine_allocation::boxed(Self {
            slots,
            claims,
            claimed: AtomicUsize::new(0),
        })
        .ok()
    }

    /// Hook path: whether `holder` is in the set; false only when full.
    fn insert(&self, holder: usize) -> bool {
        let len = self.slots.len();
        let mut index = crate::table_hash::slot(holder as u64, len);
        for _ in 0..len {
            match self.slots[index].compare_exchange(0, holder, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    let at = self.claimed.fetch_add(1, Ordering::Relaxed);
                    if let Some(claim) = self.claims.get(at) {
                        claim.store(index, Ordering::Relaxed);
                    }
                    return true;
                }
                Err(current) if current == holder => return true,
                Err(_) => index = (index + 1) & (len - 1),
            }
        }
        false
    }
}

/// Writer-side interval state: a writer leases `active` before testing
/// `closed`; a take closes `closed` before testing `active` (all SeqCst), so
/// either the take sees the lease or the writer sees the gate and loses the
/// interval.
struct Touched {
    active: AtomicUsize,
    closed: AtomicBool,
    /// A write since the last take that the set does not name.
    lost: AtomicBool,
    observing: AtomicBool,
    set: AtomicPtr<Set>,
    /// The installed set's slots, so status never reads a set a take may free.
    slots: AtomicUsize,
    calls: AtomicU64,
    lost_calls: AtomicU64,
}

impl Touched {
    const fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            lost: AtomicBool::new(true),
            observing: AtomicBool::new(false),
            set: AtomicPtr::new(std::ptr::null_mut()),
            slots: AtomicUsize::new(0),
            calls: AtomicU64::new(0),
            lost_calls: AtomicU64::new(0),
        }
    }

    /// Hook path, inside the caller's lease.
    fn notify(&self, holder: usize) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let recorded = self.observing.load(Ordering::SeqCst)
            && !self.closed.load(Ordering::SeqCst)
            && !crate::world_lifetime::invalidating()
            // SAFETY: a published set is replaced only by an exclusive take,
            // which no lease spans, and freed only after that replacement.
            && unsafe { self.set.load(Ordering::SeqCst).as_ref() }
                .is_some_and(|set| set.insert(holder));
        if !recorded {
            self.lost_calls.fetch_add(1, Ordering::Relaxed);
            self.lost.store(true, Ordering::SeqCst);
        }
    }

    /// Worker, while the game thread is parked: append the holders touched
    /// since the last take to `out` and clear the set, installing `offered`
    /// once it is empty. Returns whether every write of the interval is named.
    fn take(
        &self,
        out: &mut Vec<usize>,
        offered: &mut Option<Box<Set>>,
    ) -> Result<bool, &'static str> {
        self.closed.store(true, Ordering::SeqCst);
        if self.active.load(Ordering::SeqCst) != 0 {
            // A lease spans the gate: leave the set to the next exclusive take.
            self.lost.store(true, Ordering::SeqCst);
            self.closed.store(false, Ordering::SeqCst);
            return Ok(false);
        }
        let result = (|| {
            // SAFETY: the gate is closed and no lease remains.
            if let Some(set) = unsafe { self.set.load(Ordering::SeqCst).as_ref() } {
                let claimed = set.claimed.swap(0, Ordering::Relaxed).min(set.claims.len());
                engine_allocation::reserve(out, claimed)
                    .map_err(|_| "air region combat touched-holder allocation failed")?;
                for claim in &set.claims[..claimed] {
                    if let Some(slot) = set.slots.get(claim.load(Ordering::Relaxed)) {
                        let holder = slot.swap(0, Ordering::Relaxed);
                        if holder != 0 {
                            out.push(holder);
                        }
                    }
                }
            }
            if let Some(next) = offered.take() {
                // SAFETY: the gate is closed and no lease remains.
                unsafe { self.replace(Some(next)) };
            }
            Ok(())
        })();
        // Without installed writers nothing reaches the set, so no interval is named.
        let complete = result.is_ok()
            && !self.lost.swap(false, Ordering::SeqCst)
            && self.observing.load(Ordering::SeqCst);
        self.closed.store(false, Ordering::SeqCst);
        result.map(|()| complete)
    }

    /// The installed set's slots.
    fn capacity(&self) -> usize {
        self.slots.load(Ordering::SeqCst)
    }

    /// Publishes `next`, or no set, and frees the set it replaces.
    ///
    /// # Safety
    /// Nothing may hold the replaced set: no lease spans the call and no hook can start one.
    unsafe fn replace(&self, next: Option<Box<Set>>) {
        self.slots.store(
            next.as_ref().map_or(0, |set| set.slots.len()),
            Ordering::SeqCst,
        );
        let next = next.map_or(std::ptr::null_mut(), Box::into_raw);
        let previous = self.set.swap(next, Ordering::SeqCst);
        if !previous.is_null() {
            // SAFETY: published by `Box::into_raw`; the caller guarantees nothing holds it.
            drop(unsafe { Box::from_raw(previous) });
        }
    }
}

/// One held holder: its row (key, owner and records as last read) and the
/// address it was read at.
struct Held {
    address: usize,
    row: AirRegionCombatRow,
}

#[derive(Default)]
struct Store {
    /// Every holder of the latest complete capture, in key order, including
    /// holders without records.
    holders: Vec<Held>,
    by_address: HashMap<usize, usize>,
    touched: Vec<usize>,
    /// The sample hour of the latest complete capture.
    hour: Option<i32>,
    /// The table published for `holders`; rebuilt only when a row changed.
    published: Option<(Arc<TableRows>, usize)>,
    changed: bool,
    /// A larger touched-holder set for the next take.
    offered: Option<Box<Set>>,
}

fn convert(record: native::Record) -> AirCombatRecord {
    let [first, second] = record.damage;
    let damage = |entries: Vec<native::Damage>| {
        entries
            .into_iter()
            .map(|entry| AirCombatDamage {
                cause: entry.cause,
                amount: entry.amount,
            })
            .collect()
    };
    AirCombatRecord {
        wing_id: record.wing_id,
        date: record.date,
        planes: record.planes,
        mission_mask: record.mission_mask,
        selection: record.selection,
        classification: record.classification,
        copied: record.copied,
        attributed_country: record.attributed_country,
        first_damage: damage(first),
        second_damage: damage(second),
    }
}

fn row(country: u32, region: u32, holder: native::Holder) -> AirRegionCombatRow {
    let [own, foreign] = holder.sides;
    AirRegionCombatRow {
        country,
        region,
        owner: holder.owner,
        own: own.records.into_iter().map(convert).collect(),
        foreign: foreign.records.into_iter().map(convert).collect(),
    }
}

/// The start of `records`' maximal suffix dated at or after `since`.
fn suffix_start(records: &[AirCombatRecord], since: i32) -> usize {
    records
        .iter()
        .rposition(|record| record.date < since)
        .map_or(0, |index| index + 1)
}

/// A touched vector's state: `held` less the leading records the daily
/// prune removed, then `tail`, the native records from `first` on (those
/// dated at or after `since`). The native records before `first` are the held
/// records before their own suffix dated at or after `since`, less that
/// leading run; fewer held than native ones is a write the store missed.
fn splice(
    held: &mut Vec<AirCombatRecord>,
    since: i32,
    first: usize,
    tail: impl Iterator<Item = AirCombatRecord>,
) -> Option<bool> {
    let pruned = suffix_start(held, since).checked_sub(first)?;
    // The native tail is dated at or after since. Removing an older prefix
    // therefore changes the row even if the replacement tail has equal values.
    let mut changed = pruned != 0;
    held.drain(..pruned);
    let mut len = first;
    for record in tail {
        if let Some(previous) = held.get_mut(len) {
            if *previous != record {
                *previous = record;
                changed = true;
            }
        } else {
            held.push(record);
            changed = true;
        }
        len += 1;
    }
    changed |= held.len() != len;
    held.truncate(len);
    Some(changed)
}

/// Why an hour cannot carry: a capture-local difference poisons the family.
enum Refusal {
    Native(&'static str),
    Mismatch,
}

impl From<&'static str> for Refusal {
    fn from(reason: &'static str) -> Self {
        Self::Native(reason)
    }
}

impl Store {
    fn index(&mut self) {
        self.by_address.clear();
        for (index, held) in self.holders.iter().enumerate() {
            self.by_address.insert(held.address, index);
        }
    }

    /// Read every holder of every country air object.
    fn full(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let mut holders = Vec::new();
        native::visit_holders(memory, base, state, |root| {
            let holder = native::read_holder(memory, base, root.address, None)?;
            engine_allocation::reserve(&mut holders, 1)
                .map_err(|_| "air region combat allocation failed")?;
            holders.push(Held {
                address: root.address,
                row: row(root.country, root.region, holder),
            });
            Ok(())
        })?;
        holders.sort_by_key(|held| (held.row.country, held.row.region));
        if holders.windows(2).any(|pair| {
            (pair[0].row.country, pair[0].row.region) == (pair[1].row.country, pair[1].row.region)
        }) {
            return Err("air region combat holder key repeats");
        }
        self.holders = holders;
        self.index();
        self.changed = true;
        Ok(())
    }

    /// Read the touched holder at `index` from `since` on and splice the
    /// older records from its held row by the native counts.
    fn refresh(
        &mut self,
        memory: &impl Memory,
        base: usize,
        index: usize,
        since: i32,
    ) -> Result<(), Refusal> {
        let held = &mut self.holders[index];
        let holder = native::read_holder(memory, base, held.address, Some(since))?;
        if holder.owner != held.row.owner {
            return Err(Refusal::Mismatch);
        }
        COUNTS[1].fetch_add(1, Ordering::Relaxed);
        let [own, foreign] = holder.sides;
        for (side, records) in [(own, &mut held.row.own), (foreign, &mut held.row.foreign)] {
            COUNTS[2].fetch_add(side.records.len() as u64, Ordering::Relaxed);
            let first = side.first as usize;
            let tail = side.records.into_iter().map(convert);
            let Some(changed) = splice(records, since, first, tail) else {
                return Err(Refusal::Mismatch);
            };
            self.changed |= changed;
        }
        Ok(())
    }

    /// A touched holder the store does not hold: located through its owner's
    /// table and read whole.
    fn discover(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        address: usize,
    ) -> Result<(), Refusal> {
        let holder = native::read_holder(memory, base, address, None)?;
        let Some(root) = native::locate(memory, base, state, holder.owner, address)? else {
            return Err(Refusal::Mismatch);
        };
        COUNTS[3].fetch_add(1, Ordering::Relaxed);
        let row = row(root.country, root.region, holder);
        let key = (row.country, row.region);
        match self
            .holders
            .binary_search_by_key(&key, |held| (held.row.country, held.row.region))
        {
            // The table slot now holds another holder: the held one is gone.
            Ok(index) => self.holders[index] = Held { address, row },
            Err(index) => self.holders.insert(index, Held { address, row }),
        }
        self.index();
        self.changed = true;
        Ok(())
    }

    /// The hour's carry: the touched holders are read, the rest carried.
    fn carry(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        since: i32,
        touched: &[usize],
    ) -> Result<(), Refusal> {
        for &address in touched {
            match self.by_address.get(&address) {
                Some(&index) => self.refresh(memory, base, index, since)?,
                None => self.discover(memory, base, state, address)?,
            }
        }
        COUNTS[4].fetch_add(
            (self.holders.len() as u64).saturating_sub(touched.len() as u64),
            Ordering::Relaxed,
        );
        Ok(())
    }

    /// The table of the held holders with records, rebuilt when one changed.
    fn publish(&mut self) -> Result<(Arc<TableRows>, usize), &'static str> {
        if !self.changed
            && let Some((rows, len)) = &self.published
        {
            return Ok((Arc::clone(rows), *len));
        }
        let failed = |_| "air region combat allocation failed";
        let len = self
            .holders
            .iter()
            .filter(|held| !held.row.own.is_empty() || !held.row.foreign.is_empty())
            .count();
        let mut rows = engine_allocation::vec(len).map_err(failed)?;
        for held in &self.holders {
            if !held.row.own.is_empty() || !held.row.foreign.is_empty() {
                rows.push(held.row.clone());
            }
        }
        let rows = engine_allocation::boxed(TableRows::AirRegionCombat(rows))
            .map(Arc::from)
            .map_err(failed)?;
        self.published = Some((Arc::clone(&rows), len));
        self.changed = false;
        Ok((rows, len))
    }

    /// A touched-holder set for every held holder to be touched in one hour.
    fn offer(&mut self) {
        if TOUCHED.capacity() < self.holders.len().saturating_mul(2).max(MIN_SLOTS)
            && self.offered.is_none()
        {
            self.offered = Set::new(self.holders.len());
        }
    }

    fn forget(&mut self) {
        self.holders.clear();
        self.by_address.clear();
        self.hour = None;
        self.published = None;
        self.changed = true;
    }
}

/// The FinalInputs worker, once an hour while the game thread is parked: the
/// AirRegionCombat table at `hour` and its row count.
pub fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    hour: i32,
    force_full: bool,
) -> Result<(Arc<TableRows>, usize), &'static str> {
    let Ok(mut guard) = STORE.try_lock() else {
        TOUCHED.lost.store(true, Ordering::SeqCst);
        return Err("air region combat capture overlaps its store");
    };
    let store = guard.get_or_insert_with(Store::default);
    let mut touched = std::mem::take(&mut store.touched);
    touched.clear();
    let observed = TOUCHED.take(&mut touched, &mut store.offered);
    let full_cause = match (&observed, store.hour) {
        (_, None) => Some(0),
        _ if force_full => Some(1),
        (Ok(false) | Err(_), _) => Some(2),
        _ if POISONED.load(Ordering::SeqCst) || !cfg!(feature = "air_record_changes") => Some(3),
        _ => None,
    };
    let result = match (full_cause, store.hour) {
        (None, Some(since)) => match store.carry(memory, base, state, since, &touched) {
            Ok(()) => {
                COUNTS[0].fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(Refusal::Mismatch) => {
                POISONED.store(true, Ordering::SeqCst);
                FULL[4].fetch_add(1, Ordering::Relaxed);
                store.full(memory, base, state)
            }
            Err(Refusal::Native(reason)) => Err(reason),
        },
        (cause, _) => {
            FULL[cause.unwrap_or(4)].fetch_add(1, Ordering::Relaxed);
            store.full(memory, base, state)
        }
    };
    store.touched = touched;
    match result.and_then(|()| store.publish()) {
        Ok(published) => {
            store.hour = Some(hour);
            store.offer();
            Ok(published)
        }
        Err(reason) => {
            store.forget();
            store.offer();
            Err(reason)
        }
    }
}

/// Notifications running now.
pub fn active() -> usize {
    TOUCHED.active.load(Ordering::SeqCst)
}

pub fn status() -> impl serde::Serialize {
    let full = FULL.each_ref().map(|value| value.load(Ordering::Relaxed));
    let counts = COUNTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "air_record_changes"),
        "poisoned": POISONED.load(Ordering::SeqCst),
        "calls": TOUCHED.calls.load(Ordering::Relaxed),
        "unrecorded_calls": TOUCHED.lost_calls.load(Ordering::Relaxed),
        "set_slots": TOUCHED.capacity(),
        "carried_hours": counts[0],
        "touched_holders_read": counts[1],
        "touched_records_read": counts[2],
        "new_holders": counts[3],
        "carried_holders": counts[4],
        "full_hours": {
            "first": full[0],
            "invalidation": full[1],
            "lost_interval": full[2],
            "poisoned": full[3],
            "difference": full[4],
        },
        "scope": "cumulative since attach; calls are update_record entries (unrecorded ones lose their interval, and that hour reads every holder); a carried hour reads each touched holder's records dated from the previous capture's hour and carries the rest; a touched holder that cannot be spliced or located disables reuse (difference)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}

impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "air_record_changes") {
            let site = &MUTATIONS[0];
            if !crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel) {
                return Err("air record hook sentinel differs");
            }
            let set = Set::new(0).ok_or("air record touched-holder allocation failed")?;
            // SAFETY: no hook is installed yet, so nothing holds the previous set.
            unsafe { TOUCHED.replace(Some(set)) };
            let hook = unsafe {
                Hook::new(
                    base + site.rva,
                    update_record as *const () as usize,
                    site.sentinel,
                )
            }
            .map_err(|_| "native hook construction failed")?;
            ORIGINAL.store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(Self { hooks })
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            TOUCHED.observing.store(false, Ordering::SeqCst);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared_prepared(&mut self.hooks, enabled, || {
            // Writes ran unobserved before enabling and after disabling.
            TOUCHED.lost.store(true, Ordering::SeqCst);
        })
        .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        TOUCHED
            .observing
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the set is dropped only once the
        // hooks were never enabled, or were removed and drained.
        unsafe { TOUCHED.replace(None) };
        if let Ok(mut store) = STORE.lock() {
            *store = None;
        }
    }
}

/// The writer lease spans the original: a take never meets a running write.
struct Lease;
impl Drop for Lease {
    fn drop(&mut self) {
        TOUCHED.active.fetch_sub(1, Ordering::SeqCst);
    }
}

unsafe extern "system" fn update_record(
    holder: usize,
    wing: usize,
    cause: u32,
    amount: i64,
    classification: u8,
    selection: u8,
) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::AirRecordChanges);
    TOUCHED.active.fetch_add(1, Ordering::SeqCst);
    let _lease = Lease;
    TOUCHED.notify(holder);
    let original: unsafe extern "system" fn(usize, usize, u32, i64, u8, u8) =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder, wing, cause, amount, classification, selection) })
}

#[cfg(test)]
mod tests;
