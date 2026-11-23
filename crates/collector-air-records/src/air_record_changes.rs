use crate::hook::Hook;
use engine_air_history::{air_record_mutations::MUTATIONS, cair_region_combat_data as native};
use engine_native_memory::Memory;
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
static FULL: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static COUNTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

static EQUIPMENT_REFUSALS: [AtomicU64; native::EQUIPMENT_REFUSALS.len()] =
    [const { AtomicU64::new(0) }; native::EQUIPMENT_REFUSALS.len()];

const MIN_SLOTS: usize = 1024;

struct Set {
    slots: Vec<AtomicUsize>,
    claims: Vec<AtomicUsize>,
    claimed: AtomicUsize,
}

impl Set {
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

struct Touched {
    active: AtomicUsize,
    closed: AtomicBool,
    lost: AtomicBool,
    observing: AtomicBool,
    set: AtomicPtr<Set>,
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

    fn notify(&self, holder: usize) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let recorded = self.observing.load(Ordering::SeqCst)
            && !self.closed.load(Ordering::SeqCst)
            && !crate::world_lifetime::invalidating()
            // SAFETY: a set is freed only once every callback that could have loaded it has
            // drained: a take replaces it with the gate closed, teardown with the hooks quiescent.
            && unsafe { self.set.load(Ordering::SeqCst).as_ref() }
                .is_some_and(|set| set.insert(holder));
        if !recorded {
            self.lost_calls.fetch_add(1, Ordering::Relaxed);
            self.lost.store(true, Ordering::SeqCst);
        }
    }

    fn take(
        &self,
        out: &mut Vec<usize>,
        offered: &mut Option<Box<Set>>,
    ) -> Result<bool, &'static str> {
        self.closed.store(true, Ordering::SeqCst);
        if self.active.load(Ordering::SeqCst) != 0 {
            self.lost.store(true, Ordering::SeqCst);
            self.closed.store(false, Ordering::SeqCst);
            return Ok(false);
        }
        let result = (|| {
            // SAFETY: the gate is closed and every callback that could have loaded this set before
            // the close has drained; a later one sees `closed`, or loads the replacement once the
            // gate reopens.
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
                // SAFETY: the gate is closed and every callback that could have loaded the old set
                // before the close has drained; a later one sees `closed`, or loads the new set once
                // the gate reopens.
                unsafe { self.replace(Some(next)) };
            }
            Ok(())
        })();
        let complete = result.is_ok()
            && !self.lost.swap(false, Ordering::SeqCst)
            && self.observing.load(Ordering::SeqCst);
        self.closed.store(false, Ordering::SeqCst);
        result.map(|()| complete)
    }

    fn capacity(&self) -> usize {
        self.slots.load(Ordering::SeqCst)
    }

    /// Publishes `next`, or no set, and frees the set it replaces.
    ///
    /// # Safety
    /// No callback can still access the replaced set. Hook quiescence guarantees that at setup and
    /// teardown; during capture the closed gate does, since every lease taken before it closed has
    /// drained and a callback that takes one later sees `closed`, or loads the new set once the
    /// gate reopens.
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

struct Held {
    address: usize,
    row: AirRegionCombatRow,
}

#[derive(Default)]
struct Store {
    holders: Vec<Held>,
    by_address: HashMap<usize, usize>,
    touched: Vec<usize>,
    hour: Option<i32>,
    published: Option<(Arc<TableRows>, usize)>,
    changed: bool,
    offered: Option<Box<Set>>,
    reader: native::Reader,
    read: native::Holder,
}

fn convert(holder: &native::Holder, record: &native::Record) -> AirCombatRecord {
    let equipment = holder.equipment(record);
    if let Err(index) = record.equipment {
        EQUIPMENT_REFUSALS[usize::from(index)].fetch_add(1, Ordering::Relaxed);
    }
    let damage = |channel: usize| {
        holder
            .damage(record, channel)
            .iter()
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
        first_damage: damage(0),
        second_damage: damage(1),
        equipment: replay_data::rows::Recorded::Value(equipment.ok().map(|entries| {
            entries
                .iter()
                .map(|entry| replay_data::rows::AirCombatEquipment {
                    variant: entry.variant,
                    amount_raw: entry.amount_raw,
                })
                .collect()
        })),
    }
}

fn row(country: u32, region: u32, holder: &native::Holder) -> AirRegionCombatRow {
    let [own, foreign] = &holder.sides;
    AirRegionCombatRow {
        country,
        region,
        owner: holder.owner,
        own: Arc::new(
            own.records
                .iter()
                .map(|record| convert(holder, record))
                .collect(),
        ),
        foreign: Arc::new(
            foreign
                .records
                .iter()
                .map(|record| convert(holder, record))
                .collect(),
        ),
    }
}

fn suffix_start(records: &[AirCombatRecord], since: i32) -> usize {
    records
        .iter()
        .rposition(|record| record.date < since)
        .map_or(0, |index| index + 1)
}

fn splice(
    held: &mut Arc<Vec<AirCombatRecord>>,
    since: i32,
    first: usize,
    tail: impl Iterator<Item = AirCombatRecord>,
) -> Option<bool> {
    let pruned = suffix_start(&held[..], since).checked_sub(first)?;
    let mut tail = tail.peekable();
    let mut len = first;
    if pruned == 0 {
        while tail
            .next_if(|record| held.get(len) == Some(record))
            .is_some()
        {
            len += 1;
        }
        if tail.peek().is_none() && held.len() == len {
            return Some(false);
        }
    }
    let records = Arc::make_mut(held);
    records.drain(..pruned);
    for record in tail {
        if let Some(previous) = records.get_mut(len) {
            if *previous != record {
                *previous = record;
            }
        } else {
            records.push(record);
        }
        len += 1;
    }
    records.truncate(len);
    Some(true)
}

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

    fn full(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let mut holders = Vec::new();
        let (reader, read) = (&mut self.reader, &mut self.read);
        native::visit_holders(memory, base, state, |root| {
            native::read_holder_into(memory, base, root.address, None, reader, read)?;
            engine_allocation::reserve(&mut holders, 1)
                .map_err(|_| "air region combat allocation failed")?;
            holders.push(Held {
                address: root.address,
                row: row(root.country, root.region, read),
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

    fn refresh(
        &mut self,
        memory: &impl Memory,
        base: usize,
        index: usize,
        since: i32,
    ) -> Result<(), Refusal> {
        let held = &mut self.holders[index];
        let holder = &mut self.read;
        native::read_holder_into(
            memory,
            base,
            held.address,
            Some(since),
            &mut self.reader,
            holder,
        )?;
        if holder.owner != held.row.owner {
            return Err(Refusal::Mismatch);
        }
        COUNTS[1].fetch_add(1, Ordering::Relaxed);
        let holder = &*holder;
        for (side, records) in holder
            .sides
            .iter()
            .zip([&mut held.row.own, &mut held.row.foreign])
        {
            COUNTS[2].fetch_add(side.records.len() as u64, Ordering::Relaxed);
            let first = side.first as usize;
            let tail = side.records.iter().map(|record| convert(holder, record));
            let Some(changed) = splice(records, since, first, tail) else {
                return Err(Refusal::Mismatch);
            };
            self.changed |= changed;
        }
        Ok(())
    }

    fn discover(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        address: usize,
    ) -> Result<(), Refusal> {
        native::read_holder_into(
            memory,
            base,
            address,
            None,
            &mut self.reader,
            &mut self.read,
        )?;
        let Some(root) = native::locate(memory, base, state, self.read.owner, address)? else {
            return Err(Refusal::Mismatch);
        };
        COUNTS[3].fetch_add(1, Ordering::Relaxed);
        let row = row(root.country, root.region, &self.read);
        let key = (row.country, row.region);
        match self
            .holders
            .binary_search_by_key(&key, |held| (held.row.country, held.row.region))
        {
            Ok(index) => self.holders[index] = Held { address, row },
            Err(index) => self.holders.insert(index, Held { address, row }),
        }
        self.index();
        self.changed = true;
        Ok(())
    }

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
    store.reader.forget_variants();
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

pub fn active() -> usize {
    TOUCHED.active.load(Ordering::SeqCst)
}


pub struct Hooks {
    hooks: Vec<Hook>,
}

impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "air_record_changes") {
            let code = engine_sites::hook(base, MUTATIONS[0].rva)
                .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                .inspect_err(|cause| {
                    crate::hook::installation::refused(
                        crate::hook::installation::Kind::Sentinel,
                        None,
                        cause,
                    )
                })?;
            let set = Set::new(0).ok_or("air record touched-holder allocation failed")?;
            // SAFETY: no hook is installed yet, so nothing holds the previous set.
            unsafe { TOUCHED.replace(Some(set)) };
            let hook = unsafe {
                Hook::new(code.0, update_record as *const () as usize, code.1)
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
        if !Hook::set_family_enabled_shared_prepared(&mut self.hooks, enabled, || {
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
