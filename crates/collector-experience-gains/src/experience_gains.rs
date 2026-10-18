//! Exact positive awards at the six native experience bank additions. Storage
//! grows from the live country census off-thread; observers never allocate,
//! wait, take the store lock or call a game getter. Awards are summed per
//! authenticated owner and branch in lock-free accumulators, so concurrent
//! native writers neither exclude one another nor change a total by their
//! order. A failed interval is unavailable; a fresh authenticated boundary can
//! start the next complete interval.
use crate::hook::{Hook, Registers, Tap};
type Clock = engine_native::cgame_state::Clock<u64>;
use engine_events::experience_gain_sites as native;
use engine_native::{Memory, ccountry, read};
use replay_data::rows::CountryExperienceGainRow;
use std::{
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

/// Writer-side interval state shared without the store lock. A writer leases
/// `active` before it tests `closed`; exclusive drain/boundary work closes
/// `closed` before it tests `active` (all SeqCst). Either the reset sees the
/// lease, or the writer sees the gate and refuses without touching the table.
struct Lanes {
    active: AtomicUsize,
    poisoned: AtomicBool,
    closed: AtomicBool,
    table: AtomicPtr<Table>,
}
impl Lanes {
    const fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            poisoned: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            table: AtomicPtr::new(std::ptr::null_mut()),
        }
    }
    /// True when no writer holds a lease; new writers stay out until `open`.
    fn close(&self) -> bool {
        self.closed.store(true, Ordering::SeqCst);
        self.active.load(Ordering::SeqCst) == 0
    }
    fn open(&self) {
        self.closed.store(false, Ordering::SeqCst);
    }
    fn publish(&self, table: &Table) {
        self.table
            .store(std::ptr::from_ref(table).cast_mut(), Ordering::SeqCst);
    }
    /// One native award, inside the caller's `active` lease.
    fn record(
        &self,
        memory: &impl Memory,
        base: usize,
        registers: &Registers,
        site: usize,
    ) -> Result<(), &'static str> {
        let source = &native::SITES[site];
        let amount_pointer = if source.air_argument {
            registers.rsi
        } else {
            registers.rdi
        };
        let amount = read::<i64>(memory, amount_pointer)
            .ok_or("experience accepted adjustment unreadable")?;
        // Spending/corrections never enter the income sum. The native gain
        // modifier has already run and clamped a negative product to zero.
        if amount <= 0 {
            return Ok(());
        }
        let (owner, _) = native::owner(memory, base, registers.rbx)?;
        // An award racing the hourly reset belongs to neither side of it.
        if self.closed.load(Ordering::SeqCst) {
            return Err("experience gain writer overlaps drain");
        }
        let table = self.table.load(Ordering::SeqCst);
        if table.is_null() {
            return Err("experience gain storage not prepared");
        }
        // SAFETY: a published table is replaced only while the gate is closed
        // with no lease, and released only after hook quiescence.
        unsafe { &*table }.add(owner, source.branch, amount)
    }
}
static LIVE: Lanes = Lanes::new();

const VACANT_INSTANCE: u64 = u64::MAX;

struct Slot {
    pointer: AtomicUsize,
    instance: AtomicU64,
    gains: [AtomicI64; 3],
}
impl Slot {
    fn vacant() -> Self {
        Self {
            pointer: AtomicUsize::new(0),
            instance: AtomicU64::new(VACANT_INSTANCE),
            gains: Default::default(),
        }
    }
}

/// Open-addressed per-owner sums. A slot's pointer and instance words each
/// leave vacancy at most once per interval and never change again until an
/// exclusive reset, so writers racing for one owner settle on the same slot by
/// compare-exchange alone, and additions commute. `claims` lists the slots
/// taken this interval so a reset touches only those.
struct Table {
    slots: Vec<Slot>,
    claims: Vec<AtomicUsize>,
    claimed: AtomicUsize,
}
impl Table {
    /// At most half full at the live census. Births and replacements within
    /// one interval take further slots; only beyond every slot does a writer
    /// refuse.
    fn slots_for(countries: usize) -> Result<usize, &'static str> {
        if countries == 0 {
            return Ok(0);
        }
        countries
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or("experience gain allocation failed")
    }
    fn new(slots: usize) -> Result<Self, &'static str> {
        let failed = |_| "experience gain allocation failed";
        let mut owners = engine_allocation::vec(slots).map_err(failed)?;
        owners.resize_with(slots, Slot::vacant);
        let mut claims = engine_allocation::vec(slots).map_err(failed)?;
        claims.resize_with(slots, AtomicUsize::default);
        Ok(Self {
            slots: owners,
            claims,
            claimed: AtomicUsize::new(0),
        })
    }
    fn bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.slots) + crate::raw_memory::vector_bytes(&self.claims)
    }
    /// Callers ensure a nonempty, power-of-two table. Countries allocated
    /// one after another spread like random keys (`table_hash`), so probe
    /// chains stay short at any object size.
    fn start(&self, owner: native::Owner) -> usize {
        let key = owner.pointer as u64 ^ u64::from(owner.instance).rotate_left(32);
        crate::table_hash::slot(key, self.slots.len())
    }
    /// Lock-free; each probe is one or two compare-exchanges. `native::owner`
    /// never yields the null owner pointer that marks a vacant slot.
    fn add(&self, owner: native::Owner, branch: usize, amount: i64) -> Result<(), &'static str> {
        if amount <= 0 {
            return Ok(());
        }
        if self.slots.is_empty() {
            return Err("experience gain country storage overflow");
        }
        let instance = u64::from(owner.instance);
        let mut index = self.start(owner);
        for _ in 0..self.slots.len() {
            let slot = &self.slots[index];
            let pointer = match slot.pointer.compare_exchange(
                0,
                owner.pointer,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Each slot leaves vacancy once per interval: in bounds.
                    let at = self.claimed.fetch_add(1, Ordering::Relaxed);
                    if let Some(claim) = self.claims.get(at) {
                        claim.store(index, Ordering::Relaxed);
                    }
                    owner.pointer
                }
                Err(current) => current,
            };
            if pointer == owner.pointer {
                let current = match slot.instance.compare_exchange(
                    VACANT_INSTANCE,
                    instance,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => instance,
                    Err(current) => current,
                };
                if current == instance {
                    let previous = slot.gains[branch].fetch_add(amount, Ordering::Relaxed);
                    return if previous >= 0 && previous.checked_add(amount).is_some() {
                        Ok(())
                    } else {
                        Err("experience hourly gain overflow")
                    };
                }
            }
            index = (index + 1) & (self.slots.len() - 1);
        }
        Err("experience gain country storage overflow")
    }
    /// Exclusive only: no writer holds a lease.
    fn get(&self, owner: native::Owner) -> [i64; 3] {
        if self.slots.is_empty() {
            return [0; 3];
        }
        let mut index = self.start(owner);
        for _ in 0..self.slots.len() {
            let slot = &self.slots[index];
            match slot.pointer.load(Ordering::Relaxed) {
                0 => break,
                pointer
                    if pointer == owner.pointer
                        && slot.instance.load(Ordering::Relaxed) == u64::from(owner.instance) =>
                {
                    return slot
                        .gains
                        .each_ref()
                        .map(|gain| gain.load(Ordering::Relaxed));
                }
                _ => {}
            }
            index = (index + 1) & (self.slots.len() - 1);
        }
        [0; 3]
    }
    /// Exclusive only: no writer holds a lease.
    fn clear(&self) {
        let claimed = self
            .claimed
            .swap(0, Ordering::Relaxed)
            .min(self.claims.len());
        for claim in &self.claims[..claimed] {
            if let Some(slot) = self.slots.get(claim.load(Ordering::Relaxed)) {
                slot.pointer.store(0, Ordering::Relaxed);
                slot.instance.store(VACANT_INSTANCE, Ordering::Relaxed);
                for gain in &slot.gains {
                    gain.store(0, Ordering::Relaxed);
                }
            }
        }
    }
    #[cfg(test)]
    fn claimed(&self) -> usize {
        self.claimed.load(Ordering::Relaxed)
    }
}

struct Store {
    lanes: &'static Lanes,
    /// Published at `lanes.table`; replaced only by an exclusive reset.
    table: Option<Box<Table>>,
    /// A larger successor adopted at the next exclusive reset.
    offered: Option<Box<Table>>,
    /// The predecessor, freed off the game thread once the gate reopens.
    retired: Option<Box<Table>>,
    owners: Vec<Option<native::Owner>>,
    census: usize,
    clock: Option<Clock>,
    world_generation: u64,
    partial: bool,
    /// A lost interval's awards still await an exclusive reset.
    discard: bool,
    refusal: Option<&'static str>,
    charge: crate::raw_memory::Charge,
}
impl Default for Store {
    fn default() -> Self {
        Self::on(&LIVE)
    }
}
impl Store {
    fn on(lanes: &'static Lanes) -> Self {
        Self {
            lanes,
            table: None,
            offered: None,
            retired: None,
            owners: Vec::new(),
            census: 0,
            clock: None,
            world_generation: 0,
            partial: false,
            discard: false,
            refusal: None,
            charge: Default::default(),
        }
    }
    fn table_bytes(&self) -> u64 {
        [&self.table, &self.offered, &self.retired]
            .into_iter()
            .flatten()
            .map(|table| table.bytes())
            .sum()
    }
    /// Off the game thread only.
    fn prepare(&mut self, countries: usize) -> Result<(), &'static str> {
        let result = self
            .prepare_owners(countries)
            .and_then(|()| self.prepare_table(countries));
        // Growth can retain the first allocation even if the second refuses.
        self.charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(&self.owners) + self.table_bytes(),
            ..Default::default()
        });
        result
    }
    fn prepare_owners(&mut self, countries: usize) -> Result<(), &'static str> {
        if self.owners.len() < countries {
            let additional = countries - self.owners.len();
            engine_allocation::reserve_exact(&mut self.owners, additional)
                .map_err(|_| "experience gain allocation failed")?;
            self.owners.resize(countries, None);
        }
        Ok(())
    }
    /// Off the game thread only. The first table is published at once, since
    /// no writer can hold a table that does not exist yet. A larger table
    /// waits for an exclusive reset; its predecessor is freed here afterwards.
    fn prepare_table(&mut self, countries: usize) -> Result<(), &'static str> {
        drop(self.retired.take());
        let slots = Table::slots_for(countries)?;
        let ready = |table: &Option<Box<Table>>| {
            table
                .as_ref()
                .is_some_and(|table| table.slots.len() >= slots)
        };
        if ready(&self.table) || ready(&self.offered) {
            return Ok(());
        }
        let table = engine_allocation::boxed(Table::new(slots)?)
            .map_err(|_| "experience gain allocation failed")?;
        if self.table.is_none() {
            self.lanes.publish(&table);
            self.table = Some(table);
        } else {
            self.offered = Some(table);
        }
        Ok(())
    }
    /// Exclusive only: the gate is closed and no lease remains. Clears this
    /// interval's slots and adopts an offered successor once the previous
    /// predecessor has been freed.
    fn reset(&mut self) {
        if let Some(table) = &self.table {
            table.clear();
        }
        self.discard = false;
        if self.retired.is_none()
            && let Some(next) = self.offered.take()
        {
            self.lanes.publish(&next);
            self.retired = self.table.replace(next);
        }
    }
    fn boundary(&mut self, clock: Clock) {
        self.partial = self.world_generation != crate::world_lifetime::generation()
            || !self.clock.is_some_and(|previous| {
                previous.hour.checked_add(1) == Some(clock.hour)
                    && previous.campaign_id == clock.campaign_id
                    && previous.play_session_started_at == clock.play_session_started_at
            });
        self.clock = Some(clock);
        self.world_generation = crate::world_lifetime::generation();
        if self.partial {
            self.owners.fill(None);
            self.refusal = None;
            self.discard = true;
            // A writer still inside the table defers the discard to the
            // drain, which resets before its census.
            if self.lanes.close() {
                self.reset();
            }
            self.lanes.poisoned.store(false, Ordering::SeqCst);
            self.lanes.open();
        }
    }
    #[cfg(test)]
    fn add(&self, owner: native::Owner, branch: usize, amount: i64) -> Result<(), &'static str> {
        self.table
            .as_ref()
            .ok_or("experience gain storage not prepared")?
            .add(owner, branch, amount)
    }
    #[cfg(test)]
    fn table_is_empty(&self) -> bool {
        self.table.as_ref().is_none_or(|table| table.claimed() == 0)
    }
    #[cfg(test)]
    fn snapshot(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut [CountryExperienceGainRow],
    ) -> Result<usize, &'static str> {
        self.snapshot_buffer(
            memory,
            base,
            state,
            engine_native::walk::Buffer::Fixed(rows),
        )
    }
    fn snapshot_buffer(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        mut rows: engine_native::walk::Buffer<'_, CountryExperienceGainRow>,
    ) -> Result<usize, &'static str> {
        let countries = ccountry::Reader::new(memory, base, state)?;
        self.census = countries.count() as usize;
        // Pre-hourly update visits the live country roster and its count. The worker sizes row and
        // owner storage from this original census before publication; the next interval's table is
        // offered after the gate reopens.
        if rows.is_growing() {
            self.prepare_owners(countries.count() as usize)?;
        }
        rows.ensure(countries.count().saturating_sub(1) as usize)?;
        if countries.count() as usize > self.owners.len() {
            return Err("experience gain country identity storage too small");
        }
        self.owners[countries.count() as usize..].fill(None);
        let mut len = 0;
        let mut slot = 1;
        while slot < countries.count() {
            // Fixed batch width bounds stack and guarded-copy overhead, never
            // country cardinality. The callback roster supplies authentication.
            const BATCH: usize = 16;
            let count = (countries.count() - slot).min(BATCH as u32) as usize;
            let mut batch = [None; BATCH];
            let mut addresses = [0; BATCH];
            let mut words = [[0; 4]; BATCH];
            for index in 0..count {
                batch[index] = countries.at(slot + index as u32)?;
                if let Some(country) = batch[index] {
                    addresses[index] = country
                        .pointer
                        .checked_add(ccountry::ID_INSTANCE)
                        .ok_or("experience gain country instance address overflow")?;
                }
            }
            let mut destinations = words.each_mut().map(|word| word.as_mut_slice());
            for index in 0..BATCH {
                if batch[index].is_none() {
                    destinations[index] = &mut [];
                }
            }
            if memory.copy_scattered(addresses, &mut destinations) != BATCH {
                return Err("experience gain country instance unreadable");
            }
            for index in 0..count {
                let previous = &mut self.owners[(slot + index as u32) as usize];
                let Some(country) = batch[index] else {
                    *previous = None;
                    continue;
                };
                if len == rows.len() {
                    return Err("experience gain country census storage too small");
                }
                let instance = u32::from_le_bytes(words[index]);
                let key = native::Owner {
                    pointer: country.pointer,
                    instance,
                };
                let complete = u8::from(!self.partial && *previous == Some(key));
                *previous = Some(key);
                let [army_raw, navy_raw, air_raw] =
                    self.table.as_ref().map_or([0; 3], |table| table.get(key));
                rows[len] = CountryExperienceGainRow {
                    country: country.id,
                    instance,
                    complete,
                    army_raw,
                    navy_raw,
                    air_raw,
                };
                len += 1;
            }
            slot += count as u32;
        }
        // Awards to deleted native instances have no current country row.
        // They cannot be reassigned to a replacement or another raw alias.
        Ok(len)
    }

    #[cfg(test)]
    fn snapshot_interval(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut [CountryExperienceGainRow],
        refused: bool,
    ) -> Result<usize, &'static str> {
        self.snapshot_interval_buffer(
            memory,
            base,
            state,
            engine_native::walk::Buffer::Fixed(rows),
            refused,
        )
    }
    /// Awards whose lease ended before the gate closed belong to this
    /// interval; awards leased after it reopens belong to the next. A lease
    /// spanning the gate, or a writer refused at it, fails the interval.
    fn snapshot_interval_buffer(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: engine_native::walk::Buffer<'_, CountryExperienceGainRow>,
        refused: bool,
    ) -> Result<usize, &'static str> {
        if refused {
            // A new boundary cannot claim awards retained from a lost hour.
            self.owners.fill(None);
            self.partial = true;
            self.discard = true;
        }
        let lanes = self.lanes;
        let exclusive = lanes.close();
        let result = (|| {
            if !exclusive {
                return Err("experience gain writer active at drain");
            }
            if self.discard {
                self.reset();
            }
            let len = self.snapshot_buffer(memory, base, state, rows)?;
            if lanes.active.load(Ordering::SeqCst) != 0 || lanes.poisoned.load(Ordering::SeqCst) {
                return Err("experience gain writer overlaps drain");
            }
            Ok(len)
        })();
        if exclusive {
            self.reset();
        } else {
            // The leaseholder may still be adding: the next exclusive reset
            // discards this table before any census can certify it.
            self.discard = true;
        }
        lanes.open();
        result
    }
}
static STORE: Mutex<Option<Store>> = Mutex::new(None);
/// One more than the largest census already prepared; 0 before preparation.
static PREPARED: AtomicUsize = AtomicUsize::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static FIRST_REFUSAL: OnceLock<&'static str> = OnceLock::new();
static CALLS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static REFUSALS: AtomicU64 = AtomicU64::new(0);

/// Callbacks in flight, including those that record nothing.
pub fn active() -> usize {
    LIVE.active.load(Ordering::SeqCst)
}

fn refuse(reason: &'static str) {
    let _ = FIRST_REFUSAL.set(reason);
    if let Ok(mut guard) = STORE.try_lock()
        && let Some(store) = guard.as_mut()
    {
        store.refusal.get_or_insert(reason);
    }
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    LIVE.poisoned.store(true, Ordering::SeqCst);
}

/// Off the game thread, under the store lock.
fn prepare_store(store: &mut Store, countries: usize) {
    match store.prepare(countries) {
        Ok(()) => {
            PREPARED.fetch_max(countries.saturating_add(1), Ordering::AcqRel);
        }
        Err(reason) => {
            store.refusal.get_or_insert(reason);
            refuse(reason);
        }
    }
}

/// Loader/spool only. No cardinality ceiling is chosen. Demand already
/// prepared returns without the store lock, so the hourly drain rarely meets
/// it. A failed try-lock defers growth, and a writer that outgrows the current
/// capacity refuses.
pub fn prepare(countries: usize) {
    if countries < PREPARED.load(Ordering::Acquire) {
        return;
    }
    let Ok(mut guard) = STORE.try_lock() else {
        return;
    };
    prepare_store(guard.get_or_insert_with(Store::default), countries);
}

pub fn boundary(clock: Option<Clock>) {
    let Some(clock) = clock else {
        lost_drain();
        return;
    };
    let Ok(mut guard) = STORE.try_lock() else {
        lost_drain();
        return;
    };
    let Some(store) = guard.as_mut() else {
        lost_drain();
        return;
    };
    store.boundary(clock);
}

pub fn lost_drain() {
    refuse("experience gain observation lost an hourly interval");
}

unsafe extern "system-unwind" fn observe(registers: &Registers, site: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::ExperienceGains);
    struct Call;
    impl Drop for Call {
        fn drop(&mut self) {
            LIVE.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
    LIVE.active.fetch_add(1, Ordering::SeqCst);
    let _call = Call;
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    if !ENABLED.load(Ordering::SeqCst)
        || crate::world_lifetime::invalidating()
        || LIVE.poisoned.load(Ordering::SeqCst)
        || crate::allocation::failed()
    {
        return;
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    if let Err(reason) = LIVE.record(&memory, BASE.load(Ordering::Relaxed), registers, site) {
        refuse(reason);
    }
}

/// Every current country gets a row. Only the same authenticated native owner
/// at both boundaries certifies a complete interval, including exact zero.
/// A birth/replacement's first interval cannot complete a daily window.
pub fn drain(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut Vec<CountryExperienceGainRow>,
) -> Result<usize, &'static str> {
    if !ENABLED.load(Ordering::SeqCst) || UNHOOKED.get().is_some() {
        return Err("experience gain hooks unavailable");
    }
    let Ok(mut guard) = STORE.try_lock() else {
        lost_drain();
        return Err("experience gain drain overlaps storage preparation");
    };
    let Some(store) = guard.as_mut() else {
        return Err("experience gain storage not prepared");
    };
    let refused = LIVE.poisoned.swap(false, Ordering::SeqCst).then(|| {
        store
            .refusal
            .take()
            .unwrap_or("experience gain observation incomplete")
    });
    let result = store.snapshot_interval_buffer(
        memory,
        base,
        state,
        engine_native::walk::Buffer::Growing(rows),
        refused.is_some(),
    );
    if let Err(reason) = result {
        store.refusal.get_or_insert(reason);
        refuse(reason);
    }
    // This acquisition worker offers a table for the census just read and
    // frees a predecessor the reset retired; the next reset adopts the offer.
    let census = store.census;
    prepare_store(store, census);
    // A successfully reseeded census cannot repair the lost interval. Its
    // next interval can be complete, and the viewer still needs a whole day.
    result.and_then(|len| refused.map_or(Ok(len), Err))
}

pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "unhooked": UNHOOKED.get(), "enabled": ENABLED.load(Ordering::SeqCst),
        "calls": CALLS.each_ref().map(|value| value.load(Ordering::Relaxed)),
        "refusals": REFUSALS.load(Ordering::Relaxed), "refused": FIRST_REFUSAL.get(),
    })
}

/// Only once no published tap can run: writers never load the table again.
fn release(store: &Mutex<Option<Store>>) {
    let store = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(store) = &store {
        store
            .lanes
            .table
            .store(std::ptr::null_mut(), Ordering::SeqCst);
        if std::ptr::eq(store.lanes, &LIVE) {
            PREPARED.store(0, Ordering::Release);
        }
    }
    drop(store);
}

pub struct Hooks {
    taps: Vec<Tap>,
    store: &'static Mutex<Option<Store>>,
    publication_attempted: bool,
}
impl Hooks {
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let result = (|| {
            for site in &native::SITES {
                let at = base
                    .checked_add(site.rva)
                    .ok_or("experience gain site address overflow")?;
                if read::<[u8; 16]>(memory, at) != Some(site.sentinel) {
                    return Err("experience gain native sentinel mismatch");
                }
            }
            BASE.store(base, Ordering::Relaxed);
            native::SITES
                .iter()
                .enumerate()
                .map(|(index, site)| unsafe {
                    let at = base
                        .checked_add(site.rva)
                        .ok_or("experience gain site address overflow")?;
                    Tap::new(at, observe, index, site.sentinel)
                        .map_err(|_| "experience gain tap construction failed")
                })
                .collect::<Result<Vec<_>, _>>()
        })();
        Self {
            taps: result.unwrap_or_else(|reason| {
                let _ = UNHOOKED.set(reason);
                Vec::new()
            }),
            store: &STORE,
            publication_attempted: false,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !crate::hook::shared::defer_preparation(|| {
            ENABLED.store(false, Ordering::SeqCst);
        }) {
            ENABLED.store(false, Ordering::SeqCst);
        }
        let result = match crate::hook::shared::attach_outcome() {
            Some(outcome) => outcome.map_err(|_| "experience gain hook publication failed")?,
            None => {
                let mut hooks =
                    crate::hook::shared::publication_storage_attach(self.taps.len(), enabled)
                        .map_err(|_| "experience gain hook family allocation failed")?;
                hooks.extend(self.taps.iter_mut().map(Tap::publication_hook));
                // A failed batch may have published a tap; retain storage until quiet.
                self.publication_attempted |= enabled;
                Hook::set_enabled_batch_attach(&mut hooks, enabled)
                    .map_err(|_| "experience gain hook publication failed")?
            }
        };
        if result {
            ENABLED.store(enabled && !self.taps.is_empty(), Ordering::SeqCst);
        }
        Ok(result)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps.iter().flat_map(Tap::executable_ranges)
    }
    /// Runtime only, after restoring every tap and the final IP/callback quiet
    /// check, with acquisition, decode, spool and status workers joined.
    pub fn release_after_quiet(&mut self) {
        self.publication_attempted = false;
        release(self.store);
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // Bootstrap errors never published these taps. Published storage is
        // retained unless runtime explicitly proved its complete teardown.
        if !self.publication_attempted {
            release(self.store);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{country_memory::Cache, memory::SliceMemory};

    fn owner(pointer: usize, instance: u32) -> native::Owner {
        native::Owner { pointer, instance }
    }
    fn clock(hour: i32) -> Clock {
        Clock {
            game_state: 0,
            hour,
            play_time: 0,
            play_session_started_at: 1,
            seed: 0,
            campaign_id: Default::default(),
            speed: 0,
            tick_busy: 0,
        }
    }

    const STATUSES: usize = 0x8000;
    const AMOUNTS: usize = 0x9000;
    const COUNTRIES: usize = 0x10000;
    const STRIDE: usize = 0x1600;
    /// Live countries in slots 1..slots (raw id = slot, instance = 7 * slot),
    /// each with an authenticated experience status, plus award cells.
    fn native_fixture(slots: usize, amounts: &[i64]) -> Vec<u8> {
        use engine_events::ccountry_experience_status as status;
        use engine_native::cgame_state;
        const POINTERS: usize = 0x1000;
        const LINKS: usize = 0x4000;
        let mut bytes = vec![0; COUNTRIES + slots * STRIDE];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        for (offset, pointer) in [
            (cgame_state::COUNTRIES, POINTERS),
            (cgame_state::COUNTRY_LINKS, LINKS),
        ] {
            put(offset, &(pointer as u64).to_le_bytes());
            put(offset + 8, &(slots as i32).to_le_bytes());
            put(offset + 12, &(slots as i32).to_le_bytes());
        }
        for slot in 1..slots {
            let country = COUNTRIES + slot * STRIDE;
            let status = STATUSES + slot * 16;
            put(POINTERS + slot * 8, &(country as u64).to_le_bytes());
            put(LINKS + slot * 4, &(slot as u32).to_le_bytes());
            put(country, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
            put(country + 8, &(slot as u32).to_le_bytes());
            put(country + 12, &(slot as u32 * 7).to_le_bytes());
            put(
                country + ccountry::EXPERIENCE_STATUS,
                &(status as u64).to_le_bytes(),
            );
            put(status, &(status::VTABLE_RVA as u64).to_le_bytes());
            put(status + 8, &(country as u64).to_le_bytes());
        }
        for (index, amount) in amounts.iter().enumerate() {
            put(AMOUNTS + index * 8, &amount.to_le_bytes());
        }
        bytes
    }
    /// The tapped registers: RBX is the status, RDI (RSI for air) the amount.
    fn registers(slot: usize, amount: usize) -> Registers {
        let amount = AMOUNTS + amount * 8;
        Registers {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rdi: amount,
            rsi: amount,
            rbp: 0,
            rbx: STATUSES + slot * 16,
            rdx: 0,
            rcx: 0,
            rax: 0,
            flags: 0,
        }
    }
    /// The observer's own lease around one native award.
    fn award(
        lanes: &Lanes,
        memory: &impl Memory,
        slot: usize,
        amount: usize,
        site: usize,
    ) -> Result<(), &'static str> {
        lanes.active.fetch_add(1, Ordering::SeqCst);
        let result = lanes.record(memory, 0, &registers(slot, amount), site);
        lanes.active.fetch_sub(1, Ordering::SeqCst);
        result
    }

    #[test]
    fn concurrent_native_writers_sum_exactly_without_refusal_or_store_lock() {
        static LANES: Lanes = Lanes::new();
        static LOCAL_STORE: Mutex<Option<Store>> = Mutex::new(None);
        const SLOTS: usize = 17;
        const THREADS: usize = 8;
        const ROUNDS: usize = 40;
        // Zero and spending are read but never enter a sum.
        let amounts = [1_i64, 32_768, 5, 0, -9, 77_777, 3];
        let bytes = native_fixture(SLOTS, &amounts);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut cache = Cache::try_with_capacity(Cache::cells_for_countries(SLOTS, 0)).unwrap();
        let mut rows = vec![CountryExperienceGainRow::default(); SLOTS];
        let mut store = Store::on(&LANES);
        store.prepare(SLOTS).unwrap();
        store.boundary(clock(10));
        assert_eq!(
            store.snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, false),
            Ok(SLOTS - 1)
        );
        store.boundary(clock(11));
        *LOCAL_STORE.lock().unwrap() = Some(store);
        {
            // The previous observer refused every award that met another
            // writer, or the spool's preparation, holding this lock; and a
            // writer preempted inside the table holds a lease. Both persist
            // across the whole race.
            let _held = LOCAL_STORE.lock().unwrap();
            LANES.active.fetch_add(1, Ordering::SeqCst);
            let start = std::sync::Barrier::new(THREADS);
            let refused = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for thread in 0..THREADS {
                    let (start, memory, refused) = (&start, &memory, &refused);
                    scope.spawn(move || {
                        start.wait();
                        for round in 0..ROUNDS {
                            for slot in 1..SLOTS {
                                for site in 0..native::SITES.len() {
                                    let amount = (thread + round + slot + site) % amounts.len();
                                    if award(&LANES, memory, slot, amount, site).is_err() {
                                        refused.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                    });
                }
            });
            LANES.active.fetch_sub(1, Ordering::SeqCst);
            assert_eq!(refused.load(Ordering::Relaxed), 0);
            assert!(!LANES.poisoned.load(Ordering::SeqCst));
        }
        let mut guard = LOCAL_STORE.lock().unwrap();
        let store = guard.as_mut().unwrap();
        assert_eq!(
            store.snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, false),
            Ok(SLOTS - 1)
        );
        for slot in 1..SLOTS {
            let mut expected = [0_i64; 3];
            for thread in 0..THREADS {
                for round in 0..ROUNDS {
                    for (site, source) in native::SITES.iter().enumerate() {
                        let amount = amounts[(thread + round + slot + site) % amounts.len()];
                        expected[source.branch] += amount.max(0);
                    }
                }
            }
            let row = &rows[slot - 1];
            assert_eq!(
                (row.country, row.instance, row.complete),
                (slot as u32, slot as u32 * 7, 1)
            );
            assert_eq!([row.army_raw, row.navy_raw, row.air_raw], expected);
        }
        assert!(store.table_is_empty());
        drop(guard);
        release(&LOCAL_STORE);
        assert!(LANES.table.load(Ordering::SeqCst).is_null());
    }

    #[test]
    fn racing_claims_for_shared_and_replaced_owners_converge_on_one_slot_each() {
        // One pointer with two instances is a replacement within the hour.
        let owners = [
            owner(0x1000, 1),
            owner(0x1000, 2),
            owner(0x2000, 1),
            owner(0x3000, u32::MAX),
        ];
        const THREADS: usize = 8;
        const ADDS: usize = 32;
        // Eight slots for four owners: probe chains collide.
        let table = Table::new(Table::slots_for(owners.len()).unwrap()).unwrap();
        assert_eq!(table.slots.len(), 8);
        for _ in 0..200 {
            let start = std::sync::Barrier::new(THREADS);
            std::thread::scope(|scope| {
                for thread in 0..THREADS {
                    let (table, start, owners) = (&table, &start, &owners);
                    scope.spawn(move || {
                        start.wait();
                        for add in 0..ADDS {
                            for (index, owner) in owners.iter().enumerate() {
                                let amount = (thread * ADDS + add + 1) as i64;
                                table
                                    .add(*owner, (thread + add + index) % 3, amount)
                                    .unwrap();
                            }
                        }
                    });
                }
            });
            for (index, owner) in owners.iter().enumerate() {
                let mut expected = [0_i64; 3];
                for thread in 0..THREADS {
                    for add in 0..ADDS {
                        expected[(thread + add + index) % 3] += (thread * ADDS + add + 1) as i64;
                    }
                }
                assert_eq!(table.get(*owner), expected);
            }
            // No racing claim split an owner across two slots.
            assert_eq!(table.claimed(), owners.len());
            table.clear();
            assert!(owners.iter().all(|owner| table.get(*owner) == [0; 3]));
        }
    }

    #[test]
    fn hourly_gate_keeps_interval_order_and_refuses_only_awards_racing_it() {
        static LANES: Lanes = Lanes::new();
        const SLOTS: usize = 3;
        let bytes = native_fixture(SLOTS, &[5]);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut cache = Cache::try_with_capacity(Cache::cells_for_countries(SLOTS, 0)).unwrap();
        let mut rows = vec![CountryExperienceGainRow::default(); SLOTS];
        let mut store = Store::on(&LANES);
        store.prepare(SLOTS).unwrap();
        let mut drain = |store: &mut Store, refused: bool| {
            let result =
                store.snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, refused);
            (
                result,
                [0, 1].map(|row| (rows[row].complete, rows[row].army_raw)),
            )
        };
        store.boundary(clock(10));
        assert_eq!(drain(&mut store, false).0, Ok(2));
        store.boundary(clock(11));
        // Awards leased and finished before the drain belong to its interval.
        award(&LANES, &memory, 1, 0, 0).unwrap();
        assert_eq!(drain(&mut store, false), (Ok(2), [(1, 5), (1, 0)]));
        // Awards leased after it reopens belong to the next interval.
        store.boundary(clock(12));
        award(&LANES, &memory, 2, 0, 3).unwrap();
        assert_eq!(drain(&mut store, false), (Ok(2), [(1, 0), (1, 5)]));
        // A lease spanning the drain fails it, and the table is not reset
        // under the leaseholder.
        store.boundary(clock(13));
        award(&LANES, &memory, 1, 0, 0).unwrap();
        LANES.active.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            drain(&mut store, false).0,
            Err("experience gain writer active at drain")
        );
        assert!(store.discard && !store.table_is_empty());
        award(&LANES, &memory, 2, 0, 0).unwrap();
        LANES.active.fetch_sub(1, Ordering::SeqCst);
        // The refused next interval discards both before its census.
        store.boundary(clock(14));
        assert_eq!(drain(&mut store, true), (Ok(2), [(0, 0), (0, 0)]));
        store.boundary(clock(15));
        award(&LANES, &memory, 1, 0, 0).unwrap();
        assert_eq!(drain(&mut store, false), (Ok(2), [(1, 5), (1, 0)]));
        // A writer reaching the closed gate refuses without touching the
        // table; its refusal fails the drain it raced.
        store.boundary(clock(16));
        assert!(LANES.close());
        assert_eq!(
            award(&LANES, &memory, 2, 0, 0),
            Err("experience gain writer overlaps drain")
        );
        LANES.open();
        assert!(store.table_is_empty());
        LANES.poisoned.store(true, Ordering::SeqCst);
        assert_eq!(
            drain(&mut store, false).0,
            Err("experience gain writer overlaps drain")
        );
        LANES.poisoned.store(false, Ordering::SeqCst);
        // A partial boundary meeting a writer inside the table defers the
        // discard to the drain's exclusive reset.
        award(&LANES, &memory, 1, 0, 0).unwrap();
        LANES.active.fetch_add(1, Ordering::SeqCst);
        let mut reloaded = clock(17);
        reloaded.play_session_started_at += 1;
        store.boundary(reloaded);
        assert!(store.partial && store.discard && !store.table_is_empty());
        LANES.active.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(drain(&mut store, false), (Ok(2), [(0, 0), (0, 0)]));
        assert!(store.table_is_empty() && !store.discard);
        // Every reset left the gate open.
        assert!(!LANES.closed.load(Ordering::SeqCst));
    }

    #[test]
    fn store_capacity_follows_growth_reset_adoption_bootstrap_and_proven_quiet_release() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        static LANES: Lanes = Lanes::new();
        static LOCAL_STORE: Mutex<Option<Store>> = Mutex::new(None);
        let expected = |store: &Store| Capacity {
            auxiliary_vectors: vector_bytes(&store.owners) + store.table_bytes(),
            ..Default::default()
        };
        let published = |store: &Store| {
            std::ptr::eq(
                LANES.table.load(Ordering::SeqCst),
                &**store.table.as_ref().unwrap(),
            )
        };
        let mut store = Store {
            charge: Charge::empty_on(&LOCAL),
            ..Store::on(&LANES)
        };
        store.prepare(1500).unwrap();
        let initial = expected(&store);
        assert_eq!(LOCAL.snapshot().current, initial);
        assert!(published(&store));
        store.add(owner(1, 2), 0, 9).unwrap();
        store.prepare(5000).unwrap();
        let retained = expected(&store);
        assert!(retained.auxiliary_vectors > initial.auxiliary_vectors);
        assert_eq!(LOCAL.snapshot().current, retained);
        // Writers keep the published table until an exclusive reset adopts
        // the larger one; the predecessor is retained, not freed, there.
        assert_eq!(store.table.as_ref().unwrap().slots.len(), 4096);
        assert!(published(&store));
        assert!(LANES.close());
        store.reset();
        LANES.open();
        assert!(published(&store) && store.table_is_empty());
        assert_eq!(store.table.as_ref().unwrap().slots.len(), 16384);
        assert!(store.offered.is_none() && store.retired.is_some());
        assert_eq!(expected(&store), retained);
        assert_eq!(LOCAL.snapshot().current, retained);
        for pointer in 1..=5000 {
            store.add(owner(pointer, 0), 1, 1).unwrap();
        }
        // Off-thread preparation frees the predecessor.
        store.prepare(5000).unwrap();
        assert!(store.retired.is_none());
        let settled = expected(&store);
        assert!(settled.auxiliary_vectors < retained.auxiliary_vectors);
        assert_eq!(LOCAL.snapshot().current, settled);
        *LOCAL_STORE.lock().unwrap() = Some(store);
        // A failed/partial publication cannot certify quiet just by dropping
        // the hook owner. The allocation, its publication and charge remain.
        drop(Hooks {
            taps: Vec::new(),
            store: &LOCAL_STORE,
            publication_attempted: true,
        });
        assert!(LOCAL_STORE.lock().unwrap().is_some());
        assert!(!LANES.table.load(Ordering::SeqCst).is_null());
        assert_eq!(LOCAL.snapshot().current, settled);
        let mut hooks = Hooks {
            taps: Vec::new(),
            store: &LOCAL_STORE,
            publication_attempted: true,
        };
        hooks.release_after_quiet();
        assert!(LOCAL_STORE.lock().unwrap().is_none());
        assert!(LANES.table.load(Ordering::SeqCst).is_null());
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        drop(hooks);
        let mut store = Store {
            charge: Charge::empty_on(&LOCAL),
            ..Store::on(&LANES)
        };
        store.prepare(5000).unwrap();
        *LOCAL_STORE.lock().unwrap() = Some(store);
        // The same production Drop covers any bootstrap return before enable.
        drop(Hooks {
            taps: Vec::new(),
            store: &LOCAL_STORE,
            publication_attempted: false,
        });
        assert!(LOCAL_STORE.lock().unwrap().is_none());
        assert!(LANES.table.load(Ordering::SeqCst).is_null());
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        assert!(LOCAL.snapshot().peak_total_lower_bound > 0);
    }
    #[test]
    fn gain_preserves_awards_that_the_bank_or_daily_counter_would_cap() {
        let table = Table::new(Table::slots_for(1).unwrap()).unwrap();
        for amount in [9, -8, 0, 7] {
            table.add(owner(1, 2), 0, amount).unwrap();
        }
        assert_eq!(table.get(owner(1, 2)), [16, 0, 0]);
        assert_eq!(
            table.add(owner(1, 2), 0, i64::MAX),
            Err("experience hourly gain overflow")
        );
        // The refused interval is never certified; later awards keep refusing.
        assert!(table.add(owner(1, 2), 0, 1).is_err());
    }
    /// Five thousand countries allocated one after another, at object sizes
    /// from 1 KiB to 32 KiB, each sit at most a few slots past where their
    /// probe starts, so a hooked award probes about one slot. Bits 32 up of
    /// a multiplicative hash averaged 31.9 probes per owner at a 0x38C0
    /// stride (63 at worst), and over two at 32 of these strides.
    #[test]
    fn strided_countries_at_5000_countries_probe_short_chains() {
        const COUNTRIES: usize = 5_000;
        for stride in (0x400..=0x8000).step_by(0x40) {
            let table = Table::new(Table::slots_for(COUNTRIES).unwrap()).unwrap();
            let owners: Vec<_> = (0..COUNTRIES)
                .map(|index| owner(0x1_6000_0000 + index * stride, 1))
                .collect();
            for (index, &owner) in owners.iter().enumerate() {
                table.add(owner, 0, index as i64 + 1).unwrap();
            }
            let mask = table.slots.len() - 1;
            let mut probes = 0;
            for (index, &owner) in owners.iter().enumerate() {
                let start = table.start(owner);
                let distance = (0..table.slots.len())
                    .find(|&step| {
                        table.slots[(start + step) & mask]
                            .pointer
                            .load(Ordering::Relaxed)
                            == owner.pointer
                    })
                    .unwrap();
                probes += distance + 1;
                assert_eq!(table.get(owner), [index as i64 + 1, 0, 0]);
            }
            assert!(
                probes * 2 <= COUNTRIES * 3,
                "stride {stride:#x}: {probes} probes for {COUNTRIES} owners"
            );
        }
    }

    #[test]
    fn live_country_count_presizes_all_five_thousand_distinct_owners() {
        let table = Table::new(Table::slots_for(5000).unwrap()).unwrap();
        for pointer in 1..=5000 {
            table
                .add(owner(pointer, pointer as u32), 2, pointer as i64)
                .unwrap();
        }
        assert_eq!(table.claimed(), 5000);
        assert!(
            (1..=5000)
                .all(|pointer| table.get(owner(pointer, pointer as u32)) == [0, 0, pointer as i64])
        );
        // Births and replacements within the interval may take every slot;
        // only then does a writer refuse.
        let slots = table.slots.len();
        for pointer in 5001..=slots {
            table.add(owner(pointer, 0), 0, 1).unwrap();
        }
        for refused in [owner(slots + 1, 0), owner(1, 2)] {
            assert_eq!(
                table.add(refused, 0, 1),
                Err("experience gain country storage overflow")
            );
        }
        assert_eq!(table.add(owner(1, 2), 0, 0), Ok(()));
        table.clear();
        assert_eq!(table.claimed(), 0);
        assert!(table.slots.iter().all(|slot| {
            slot.pointer.load(Ordering::Relaxed) == 0
                && slot.instance.load(Ordering::Relaxed) == VACANT_INSTANCE
                && slot
                    .gains
                    .iter()
                    .all(|gain| gain.load(Ordering::Relaxed) == 0)
        }));
        let empty = Table::new(0).unwrap();
        assert_eq!(
            empty.add(owner(1, 1), 0, 1),
            Err("experience gain country storage overflow")
        );
        assert_eq!(empty.get(owner(1, 1)), [0; 3]);
    }

    #[test]
    fn hourly_census_covers_zero_and_all_5000_countries_but_not_births_or_replacements() {
        use engine_native::cgame_state;
        static LANES: Lanes = Lanes::new();
        const COUNT: usize = 5001;
        const POINTERS: usize = 0x1000;
        const LINKS: usize = 0xb000;
        const COUNTRIES: usize = 0x15000;
        let mut bytes = vec![0; COUNTRIES + COUNT * 32];
        for (offset, pointer, count) in [
            (cgame_state::COUNTRIES, POINTERS, COUNT),
            (cgame_state::COUNTRY_LINKS, LINKS, 10002),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&(pointer as u64).to_le_bytes());
            bytes[offset + 8..offset + 12].copy_from_slice(&(count as i32).to_le_bytes());
            bytes[offset + 12..offset + 16].copy_from_slice(&(count as i32).to_le_bytes());
        }
        for slot in 1..COUNT {
            let pointer = COUNTRIES + slot * 32;
            bytes[POINTERS + slot * 8..POINTERS + slot * 8 + 8]
                .copy_from_slice(&(pointer as u64).to_le_bytes());
            bytes[LINKS + slot * 4..LINKS + slot * 4 + 4]
                .copy_from_slice(&(slot as u32).to_le_bytes());
            bytes[pointer..pointer + 8]
                .copy_from_slice(&(ccountry::VTABLE_RVA as u64).to_le_bytes());
            bytes[pointer + 8..pointer + 12].copy_from_slice(&(slot as u32).to_le_bytes());
            bytes[pointer + 12..pointer + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        let last = COUNTRIES + 5000 * 32;
        bytes[last + 8..last + 12].copy_from_slice(&10001_u32.to_le_bytes());
        bytes[LINKS + 10001 * 4..LINKS + 10002 * 4].copy_from_slice(&5000_u32.to_le_bytes());
        let mut cache = Cache::try_with_capacity(Cache::cells_for_countries(COUNT, 0)).unwrap();
        let mut growing_store = Store::default();
        let mut growing_rows = Vec::new();
        let initial_memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            growing_store.snapshot(&cache.prepare(&initial_memory, 0, 0), 0, 0, &mut []),
            Err("experience gain country identity storage too small")
        );
        for _ in 0..2 {
            assert_eq!(
                growing_store
                    .snapshot_buffer(
                        &cache.prepare(&initial_memory, 0, 0),
                        0,
                        0,
                        engine_native::walk::Buffer::Growing(&mut growing_rows)
                    )
                    .unwrap(),
                5000
            );
            assert_eq!(growing_rows[4999].country, 10001);
            // An undersized returned worker frame grows within this acquisition.
            growing_rows.truncate(1);
            growing_store.owners.truncate(1);
        }
        let mut store = Store {
            owners: vec![None; COUNT],
            partial: true,
            ..Store::on(&LANES)
        };
        store.prepare_table(COUNT).unwrap();
        let mut rows = vec![CountryExperienceGainRow::default(); COUNT];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            store
                .snapshot(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows)
                .unwrap(),
            5000
        );
        assert!(rows[..5000].iter().all(|row| row.complete == 0));
        store.partial = false;
        store
            .add(
                native::Owner {
                    pointer: last,
                    instance: u32::MAX,
                },
                0,
                9,
            )
            .unwrap();
        assert_eq!(
            store
                .snapshot(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows)
                .unwrap(),
            5000
        );
        assert!(rows[..5000].iter().all(|row| row.complete == 1));
        assert_eq!((rows[4999].country, rows[4999].army_raw), (10001, 9));
        assert!(rows[..4999].iter().all(|row| row.army_raw == 0));
        // Deletion does not poison the surviving countries' exact-zero proof.
        bytes[POINTERS + 7 * 8..POINTERS + 8 * 8].fill(0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            store
                .snapshot(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows)
                .unwrap(),
            4999
        );
        assert!(rows[..4999].iter().all(|row| row.complete == 1));
        // A birth at a reused address and an instance wrapping to zero both
        // require a new beginning, even with an unchanged country raw ID.
        bytes[POINTERS + 7 * 8..POINTERS + 8 * 8]
            .copy_from_slice(&((COUNTRIES + 7 * 32) as u64).to_le_bytes());
        bytes[last + 12..last + 16].fill(0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        store
            .snapshot(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows)
            .unwrap();
        assert_eq!((rows[6].complete, rows[4999].complete), (0, 0));
        store
            .snapshot(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows)
            .unwrap();
        assert_eq!((rows[6].complete, rows[4999].complete), (1, 1));
        let short = SliceMemory {
            base: 0,
            bytes: &bytes[..last + 15],
        };
        assert_eq!(
            store.snapshot(&cache.prepare(&short, 0, 0), 0, 0, &mut rows),
            Err("experience gain country instance unreadable")
        );

        // Exercise the actual interval core without touching global observers:
        // lost publication discards awards, successful reseed starts afresh,
        // and a failed census or insufficient capacity cannot certify zero.
        let mut clock = Clock {
            game_state: 0,
            hour: 10,
            play_time: 0,
            play_session_started_at: 1,
            seed: 0,
            campaign_id: Default::default(),
            speed: 0,
            tick_busy: 0,
        };
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        store.boundary(clock);
        store
            .snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, false)
            .unwrap();
        clock.hour += 1;
        store.boundary(clock);
        let owner = native::Owner {
            pointer: last,
            instance: 0,
        };
        store.add(owner, 0, 99).unwrap();
        store
            .snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, true)
            .unwrap();
        assert!(
            rows[..5000]
                .iter()
                .all(|row| row.complete == 0 && row.army_raw == 0)
        );
        clock.hour += 1;
        store.boundary(clock);
        store.add(owner, 0, 7).unwrap();
        store
            .snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, false)
            .unwrap();
        assert!(rows[..5000].iter().all(|row| row.complete == 1));
        assert_eq!(rows[4999].army_raw, 7);
        assert!(store.table_is_empty());
        clock.hour += 1;
        store.boundary(clock);
        store.add(owner, 0, 8).unwrap();
        assert_eq!(
            store.snapshot_interval(&cache.prepare(&short, 0, 0), 0, 0, &mut rows, true,),
            Err("experience gain country instance unreadable")
        );
        assert!(store.table_is_empty());
        clock.hour += 1;
        store.boundary(clock);
        store
            .snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, true)
            .unwrap();
        assert!(rows[..5000].iter().all(|row| row.complete == 0));
        store.owners.truncate(COUNT - 1);
        assert_eq!(
            store.snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, false,),
            Err("experience gain country identity storage too small")
        );
        // Loader/spool growth supplies the full live census, without a cap.
        store.owners.resize(COUNT, None);
        clock.hour += 1;
        store.boundary(clock);
        store
            .snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, true)
            .unwrap();
        clock.hour += 1;
        store.boundary(clock);
        store
            .snapshot_interval(&cache.prepare(&memory, 0, 0), 0, 0, &mut rows, false)
            .unwrap();
        assert!(
            rows[..5000]
                .iter()
                .all(|row| row.complete == 1 && row.army_raw == 0)
        );
        clock.play_session_started_at += 1;
        store.add(owner, 0, 9).unwrap();
        store.boundary(clock);
        assert!(store.partial && store.table_is_empty());
        assert!(store.owners.iter().all(Option::is_none));
    }
}
