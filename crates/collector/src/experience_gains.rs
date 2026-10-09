//! Exact positive awards at the six native experience bank additions. Storage
//! grows from the live country census off-thread; observers never allocate,
//! wait, or call a game getter. A failed interval is unavailable; a fresh
//! authenticated boundary can start the next complete interval.
use crate::hook::{Hook, Registers, Tap};
use engine::{Memory, ccountry, cgame_state::Clock, experience_gain_sites as native, read};
use replay::rows::CountryExperienceGainRow;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Store {
    gains: HashMap<native::Owner, [i64; 3]>,
    owners: Vec<Option<native::Owner>>,
    clock: Option<Clock>,
    world_generation: u64,
    partial: bool,
    refusal: Option<&'static str>,
    charge: crate::raw_memory::Charge,
}
impl Store {
    fn prepare(&mut self, countries: usize) -> Result<(), &'static str> {
        let result = (|| {
            if self.owners.len() < countries {
                let additional = countries - self.owners.len();
                engine::allocation::reserve_exact(&mut self.owners, additional)
                    .map_err(|_| "experience gain allocation failed")?;
                self.owners.resize(countries, None);
            }
            if self.gains.capacity() < countries {
                let additional = countries.saturating_sub(self.gains.len());
                engine::allocation::reserve_map(&mut self.gains, additional)
                    .map_err(|_| "experience gain allocation failed")?;
            }
            Ok(())
        })();
        // Growth can retain the first allocation even if the second refuses.
        self.charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(&self.owners),
            index_map_entries_lower_bound: self.gains.capacity() as u64
                * std::mem::size_of::<(native::Owner, [i64; 3])>() as u64,
            ..Default::default()
        });
        result
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
            self.gains.clear();
            self.owners.fill(None);
            self.refusal = None;
        }
    }
    fn add(
        &mut self,
        owner: native::Owner,
        branch: usize,
        amount: i64,
    ) -> Result<(), &'static str> {
        if amount <= 0 {
            return Ok(());
        }
        if !self.gains.contains_key(&owner) && self.gains.len() == self.gains.capacity() {
            return Err("experience gain country storage overflow");
        }
        let gains = self.gains.entry(owner).or_default();
        gains[branch] = gains[branch]
            .checked_add(amount)
            .ok_or("experience hourly gain overflow")?;
        Ok(())
    }
    fn snapshot(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut [CountryExperienceGainRow],
    ) -> Result<usize, &'static str> {
        let countries = ccountry::Reader::new(memory, base, state)?;
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
                    self.gains.get(&key).copied().unwrap_or_default();
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

    fn snapshot_interval(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut [CountryExperienceGainRow],
        refused: bool,
        checks: (&AtomicUsize, &AtomicBool),
    ) -> Result<usize, &'static str> {
        if refused {
            // A new boundary cannot claim awards retained from a lost hour.
            self.gains.clear();
            self.owners.fill(None);
            self.partial = true;
        }
        let (active, poisoned) = checks;
        let result = (|| {
            if active.load(Ordering::SeqCst) != 0 {
                return Err("experience gain writer active at drain");
            }
            let len = self.snapshot(memory, base, state, rows)?;
            if active.load(Ordering::SeqCst) != 0 || poisoned.load(Ordering::SeqCst) {
                return Err("experience gain writer overlaps drain");
            }
            Ok(len)
        })();
        self.gains.clear();
        result
    }
}
static STORE: Mutex<Option<Store>> = Mutex::new(None);
static BASE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static FIRST_REFUSAL: OnceLock<&'static str> = OnceLock::new();
static CALLS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static REFUSALS: AtomicU64 = AtomicU64::new(0);

fn refuse(reason: &'static str) {
    let _ = FIRST_REFUSAL.set(reason);
    if let Ok(mut guard) = STORE.try_lock()
        && let Some(store) = guard.as_mut()
    {
        store.refusal.get_or_insert(reason);
    }
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    POISONED.store(true, Ordering::SeqCst);
}

/// Loader/spool only. No cardinality ceiling is chosen. A failed try-lock
/// defers growth, and a writer that outgrows the current capacity refuses.
pub fn prepare(countries: usize) {
    let Ok(mut guard) = STORE.try_lock() else {
        return;
    };
    let store = guard.get_or_insert_with(Store::default);
    if let Err(reason) = store.prepare(countries) {
        store.refusal.get_or_insert(reason);
        refuse(reason);
    }
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
    if store.partial {
        POISONED.store(false, Ordering::SeqCst);
    }
}

pub fn lost_drain() {
    refuse("experience gain observation lost an hourly interval");
}

unsafe extern "system-unwind" fn observe(registers: &Registers, site: usize) {
    struct Call;
    impl Drop for Call {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let _call = Call;
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    if !ENABLED.load(Ordering::SeqCst)
        || crate::world_lifetime::invalidating()
        || POISONED.load(Ordering::SeqCst)
        || crate::allocation::failed()
    {
        return;
    }
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let source = &native::SITES[site];
    let amount_pointer = if source.air_argument {
        registers.rsi
    } else {
        registers.rdi
    };
    let Some(amount) = read::<i64>(&memory, amount_pointer) else {
        refuse("experience accepted adjustment unreadable");
        return;
    };
    // Spending/corrections never enter the income sum. The native gain
    // modifier has already run and clamped a negative product to zero.
    if amount <= 0 {
        return;
    }
    let owner = match native::owner(&memory, BASE.load(Ordering::Relaxed), registers.rbx) {
        Ok((owner, _)) => owner,
        Err(reason) => {
            refuse(reason);
            return;
        }
    };
    let Ok(mut guard) = STORE.try_lock() else {
        refuse("experience gain writers overlap");
        return;
    };
    let Some(store) = guard.as_mut() else {
        refuse("experience gain storage not prepared");
        return;
    };
    if let Err(reason) = store.add(owner, source.branch, amount) {
        store.refusal.get_or_insert(reason);
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
    rows: &mut [CountryExperienceGainRow],
) -> Result<usize, &'static str> {
    if !ENABLED.load(Ordering::SeqCst) || UNHOOKED.get().is_some() {
        return Err("experience gain hooks unavailable");
    }
    let Ok(mut guard) = STORE.try_lock() else {
        lost_drain();
        return Err("experience gain drain overlaps writer");
    };
    let Some(store) = guard.as_mut() else {
        return Err("experience gain storage not prepared");
    };
    let refused = POISONED.swap(false, Ordering::SeqCst).then(|| {
        store
            .refusal
            .take()
            .unwrap_or("experience gain observation incomplete")
    });
    let result = store.snapshot_interval(
        memory,
        base,
        state,
        rows,
        refused.is_some(),
        (&ACTIVE, &POISONED),
    );
    if let Err(reason) = result {
        store.refusal.get_or_insert(reason);
        refuse(reason);
    }
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

pub struct Hooks {
    taps: Vec<Tap>,
    store: &'static Mutex<Option<Store>>,
    publication_attempted: bool,
}
impl Hooks {
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
        ENABLED.store(false, Ordering::SeqCst);
        let mut hooks: Vec<&mut Hook> = self.taps.iter_mut().map(Tap::publication_hook).collect();
        // A failed batch may have published a tap; retain storage until quiet.
        self.publication_attempted |= enabled;
        let result = Hook::set_enabled_batch(&mut hooks, enabled)
            .map_err(|_| "experience gain hook publication failed")?;
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
        drop(
            self.store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // Bootstrap errors never published these taps. Published storage is
        // retained unless runtime explicitly proved its complete teardown.
        if !self.publication_attempted {
            drop(
                self.store
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn store_capacity_follows_growth_clear_bootstrap_and_proven_quiet_release() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        static LOCAL_STORE: Mutex<Option<Store>> = Mutex::new(None);
        let expected = |store: &Store| Capacity {
            auxiliary_vectors: vector_bytes(&store.owners),
            index_map_entries_lower_bound: store.gains.capacity() as u64
                * std::mem::size_of::<(native::Owner, [i64; 3])>() as u64,
            ..Default::default()
        };
        let mut store = Store {
            charge: Charge::empty_on(&LOCAL),
            ..Default::default()
        };
        store.prepare(1500).unwrap();
        let initial = expected(&store);
        assert_eq!(LOCAL.snapshot().current, initial);
        store.prepare(5000).unwrap();
        let retained = expected(&store);
        assert!(retained.auxiliary_vectors > initial.auxiliary_vectors);
        assert!(retained.index_map_entries_lower_bound > initial.index_map_entries_lower_bound);
        assert_eq!(LOCAL.snapshot().current, retained);
        store
            .add(
                native::Owner {
                    pointer: 1,
                    instance: 2,
                },
                0,
                9,
            )
            .unwrap();
        store.gains.clear();
        store.owners.fill(None);
        assert_eq!(LOCAL.snapshot().current, retained);
        *LOCAL_STORE.lock().unwrap() = Some(store);
        // A failed/partial publication cannot certify quiet just by dropping
        // the hook owner. The allocation and its charge must both remain.
        drop(Hooks {
            taps: Vec::new(),
            store: &LOCAL_STORE,
            publication_attempted: true,
        });
        assert!(LOCAL_STORE.lock().unwrap().is_some());
        assert_eq!(LOCAL.snapshot().current, retained);
        let mut hooks = Hooks {
            taps: Vec::new(),
            store: &LOCAL_STORE,
            publication_attempted: true,
        };
        hooks.release_after_quiet();
        assert!(LOCAL_STORE.lock().unwrap().is_none());
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        drop(hooks);
        let mut store = Store {
            charge: Charge::empty_on(&LOCAL),
            ..Default::default()
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
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        assert!(LOCAL.snapshot().peak_total_lower_bound > 0);
    }
    #[test]
    fn gain_preserves_awards_that_the_bank_or_daily_counter_would_cap() {
        let owner = native::Owner {
            pointer: 1,
            instance: 2,
        };
        let mut store = Store::default();
        store.gains.reserve(1);
        for amount in [9, -8, 0, 7] {
            store.add(owner, 0, amount).unwrap();
        }
        assert_eq!(store.gains[&owner], [16, 0, 0]);
        assert!(store.add(owner, 0, i64::MAX).is_err());
    }
    #[test]
    fn live_country_count_presizes_all_five_thousand_distinct_owners() {
        let mut store = Store::default();
        store.gains.reserve(5000);
        for pointer in 1..=5000 {
            store
                .add(
                    native::Owner {
                        pointer,
                        instance: pointer as u32,
                    },
                    2,
                    pointer as i64,
                )
                .unwrap();
        }
        assert_eq!(store.gains.len(), 5000);
        assert_eq!(
            store.gains[&native::Owner {
                pointer: 5000,
                instance: 5000
            }][2],
            5000
        );
    }

    #[test]
    fn hourly_census_covers_zero_and_all_5000_countries_but_not_births_or_replacements() {
        use engine::{cgame_state, country_memory::Cache, memory::SliceMemory};
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
        let mut store = Store {
            owners: vec![None; COUNT],
            partial: true,
            ..Default::default()
        };
        store.gains.reserve(COUNT);
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
        let active = AtomicUsize::new(0);
        let poisoned = AtomicBool::new(false);
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
            .snapshot_interval(
                &cache.prepare(&memory, 0, 0),
                0,
                0,
                &mut rows,
                false,
                (&active, &poisoned),
            )
            .unwrap();
        clock.hour += 1;
        store.boundary(clock);
        let owner = native::Owner {
            pointer: last,
            instance: 0,
        };
        store.add(owner, 0, 99).unwrap();
        store
            .snapshot_interval(
                &cache.prepare(&memory, 0, 0),
                0,
                0,
                &mut rows,
                true,
                (&active, &poisoned),
            )
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
            .snapshot_interval(
                &cache.prepare(&memory, 0, 0),
                0,
                0,
                &mut rows,
                false,
                (&active, &poisoned),
            )
            .unwrap();
        assert!(rows[..5000].iter().all(|row| row.complete == 1));
        assert_eq!(rows[4999].army_raw, 7);
        assert!(store.gains.is_empty());
        clock.hour += 1;
        store.boundary(clock);
        store.add(owner, 0, 8).unwrap();
        assert_eq!(
            store.snapshot_interval(
                &cache.prepare(&short, 0, 0),
                0,
                0,
                &mut rows,
                true,
                (&active, &poisoned)
            ),
            Err("experience gain country instance unreadable")
        );
        assert!(store.gains.is_empty());
        clock.hour += 1;
        store.boundary(clock);
        store
            .snapshot_interval(
                &cache.prepare(&memory, 0, 0),
                0,
                0,
                &mut rows,
                true,
                (&active, &poisoned),
            )
            .unwrap();
        assert!(rows[..5000].iter().all(|row| row.complete == 0));
        store.owners.truncate(COUNT - 1);
        assert_eq!(
            store.snapshot_interval(
                &cache.prepare(&memory, 0, 0),
                0,
                0,
                &mut rows,
                false,
                (&active, &poisoned)
            ),
            Err("experience gain country identity storage too small")
        );
        // Loader/spool growth supplies the full live census, without a cap.
        store.owners.resize(COUNT, None);
        clock.hour += 1;
        store.boundary(clock);
        store
            .snapshot_interval(
                &cache.prepare(&memory, 0, 0),
                0,
                0,
                &mut rows,
                true,
                (&active, &poisoned),
            )
            .unwrap();
        clock.hour += 1;
        store.boundary(clock);
        store
            .snapshot_interval(
                &cache.prepare(&memory, 0, 0),
                0,
                0,
                &mut rows,
                false,
                (&active, &poisoned),
            )
            .unwrap();
        assert!(
            rows[..5000]
                .iter()
                .all(|row| row.complete == 1 && row.army_raw == 0)
        );
        clock.play_session_started_at += 1;
        store.add(owner, 0, 9).unwrap();
        store.boundary(clock);
        assert!(store.partial && store.gains.is_empty());
        assert!(store.owners.iter().all(Option::is_none));
    }
}
