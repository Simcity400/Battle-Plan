//! Country authentication reused only within one measured hourly callback.
//!
//! A direct `CCountry` roster census seeds exact reads of headers, pointer slots,
//! links, vtables and identity words. The seed path cannot read country scalars.
//! Each preparation discards all previous observations; verification reads the
//! original memory again. This detects changed or newly unreadable cached bytes,
//! not a transient change that is restored before verification. The caller must
//! still use the proven stable boundary and invalidate affected tables when
//! verification fails. This cache is not a synchronization mechanism.

use std::{cell::RefCell, sync::Arc};

use crate::{Memory, ccountry};

const MAX_BYTES: usize = 32;
const MAX_PROBES: usize = 16;
const VERIFY_BATCH: usize = 64;
const FILTER_WORDS: usize = 256;
const FILTER_BITS: usize = FILTER_WORDS * u64::BITS as usize;

#[derive(Clone, Copy)]
struct Entry {
    address: usize,
    len: usize,
    bytes: [u8; MAX_BYTES],
}

impl Entry {
    const EMPTY: Self = Self {
        address: 0,
        len: 0,
        bytes: [0; MAX_BYTES],
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedStatus {
    Ready,
    /// Allocate the suggested number of cells off the game thread before a
    /// later callback. The current view delegates every read to its source.
    Capacity {
        required_cells: usize,
    },
    /// No part of a failed or inconsistent census is used as cached memory.
    Unavailable {
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    Matched {
        entries: usize,
    },
    /// Seeding disabled the cache, so no cached observation was served.
    NotCached,
}

/// Reusable callback-owned storage, shared read-only with acquisition workers.
/// No lease may remain in a queued row buffer after the acquisition barrier.
pub struct Cache {
    entries: Box<[Entry]>,
    // A negative filter only: passing still requires the exact entry lookup.
    // It is rebuilt from successfully inserted keys during each preparation.
    filter: [u64; FILTER_WORDS],
    // Cheap rejection before the mixed hash. Collisions only cause extra exact
    // lookups; this bitmap never supplies bytes or authenticates a country.
    front_filter: [u64; FILTER_WORDS],
    lengths: u64,
    len: usize,
    observed_reads: usize,
    status: SeedStatus,
    // Derived results only: no additional source bytes or callback lifetime.
    // A nonzero length publishes the complete census after seed validation.
    roster: Box<[Option<ccountry::Country>]>,
    roster_len: usize,
    roster_base: usize,
    roster_game_state: usize,
}

impl Cache {
    /// Prepare one owned authentication snapshot before dispatching any readers.
    /// Every previous lease must have been returned: refusing a busy snapshot
    /// leaves its bytes and observations unchanged. Cloning the resulting Arc
    /// allocates nothing and does not extend any live game-pointer lifetime.
    pub fn prepare_shared<M: Memory>(
        storage: &mut Arc<Self>,
        source: &M,
        base: usize,
        game_state: usize,
    ) -> Result<Arc<Self>, &'static str> {
        let cache = Arc::get_mut(storage).ok_or("country snapshot still leased by a reader")?;
        cache.prepare(source, base, game_state);
        Ok(Arc::clone(storage))
    }

    /// Borrow the immutable observations from the current acquisition epoch.
    /// The owner must directly verify the snapshot after all dependent readers
    /// finish and before publishing any rows. This view cannot verify the game
    /// world's lifetime; all consumers must remain inside the hourly barrier.
    pub fn view<'a, M: Memory>(&'a self, source: &'a M) -> CountryMemory<'a, M> {
        CountryMemory {
            source,
            cache: self,
        }
    }

    /// Allocate off the game thread. Zero disables storage; other capacities
    /// round up to a power of two for bounded, division-free lookup.
    pub fn with_capacity(cells: usize) -> Self {
        let cells = if cells == 0 {
            0
        } else {
            cells.next_power_of_two()
        };
        Self {
            entries: vec![Entry::EMPTY; cells].into_boxed_slice(),
            filter: [0; FILTER_WORDS],
            front_filter: [0; FILTER_WORDS],
            lengths: 0,
            len: 0,
            observed_reads: 0,
            status: SeedStatus::Capacity { required_cells: 1 },
            roster: vec![None; cells].into_boxed_slice(),
            roster_len: 0,
            roster_base: 0,
            roster_game_state: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Owned entry/filter storage plus derived roster storage and context.
    /// Excludes allocator metadata and the shared owner's Arc control block.
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of_val(self.entries.as_ref())
            + std::mem::size_of_val(&self.filter)
            + std::mem::size_of_val(&self.front_filter)
            + std::mem::size_of_val(&self.lengths)
            + std::mem::size_of_val(self.roster.as_ref())
            + std::mem::size_of_val(&self.roster)
            + std::mem::size_of_val(&self.roster_len)
            + std::mem::size_of_val(&self.roster_base)
            + std::mem::size_of_val(&self.roster_game_state)
    }

    /// Forget every previous observation and perform one direct country census.
    /// No allocation, retry, engine call or cross-callback observation is used.
    /// A disabled result still supplies a transparent source-memory view; its
    /// seed status must be reported separately from table outcomes.
    pub fn prepare<'a, M: Memory>(
        &'a mut self,
        source: &'a M,
        base: usize,
        game_state: usize,
    ) -> CountryMemory<'a, M> {
        self.entries.fill(Entry::EMPTY);
        self.filter.fill(0);
        self.front_filter.fill(0);
        self.lengths = 0;
        self.len = 0;
        self.observed_reads = 0;
        self.status = SeedStatus::Ready;
        self.roster_len = 0;
        self.roster_base = base;
        self.roster_game_state = game_state;
        let result = {
            let recording = RecordingMemory {
                source,
                cache: RefCell::new(&mut *self),
            };
            (|| {
                let countries = ccountry::Reader::new(&recording, base, game_state)?;
                for slot in 1..countries.count() {
                    // read_at is the same census before at's pure alias
                    // filter. Retain aliases for resolve's canonical check.
                    let country = countries.read_at(slot)?;
                    if let Some(entry) = recording.cache.borrow_mut().roster.get_mut(slot as usize)
                    {
                        *entry = country;
                    }
                }
                Ok::<usize, &'static str>(countries.count() as usize)
            })()
        };
        if let Err(reason) = result {
            self.status = SeedStatus::Unavailable { reason };
        } else if matches!(self.status, SeedStatus::Capacity { .. }) {
            // Total reads are a conservative upper bound on distinct keys even
            // after storage/probes are exhausted. Also grow on a collision-only
            // miss; capacity growth belongs to the caller's off-thread work.
            let required = self
                .observed_reads
                .saturating_mul(2)
                .max(self.entries.len().saturating_mul(2))
                .max(1);
            self.status = SeedStatus::Capacity {
                required_cells: required.checked_next_power_of_two().unwrap_or(usize::MAX),
            };
        }
        if self.status == SeedStatus::Ready
            && let Ok(count) = result
            && count <= self.roster.len()
        {
            self.roster_len = count;
        }
        CountryMemory {
            source,
            cache: self,
        }
    }

    fn hash(address: usize, len: usize) -> usize {
        // Mix the complete address: native object alignment must not turn the
        // many country vtables or identity words into one collision chain.
        let mut key = (address as u64) ^ (len as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        key ^= key >> 30;
        key = key.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        key ^= key >> 27;
        key = key.wrapping_mul(0x94d0_49bb_1331_11eb);
        (key ^ (key >> 31)) as usize
    }

    #[inline]
    fn filter_bit(hash: usize) -> (usize, u64) {
        let bit = hash & (FILTER_BITS - 1);
        (
            bit / u64::BITS as usize,
            1_u64 << (bit % u64::BITS as usize),
        )
    }

    #[inline]
    fn front_filter_bit(address: usize, len: usize) -> (usize, u64) {
        // No multiplication or division on the frequent non-country miss.
        // Fold alignment bits as well as higher address bits; distribution
        // affects performance only. Insertion and lookup use the same key.
        Self::filter_bit(address ^ (address >> 3) ^ (address >> 17) ^ (len << 9))
    }

    #[inline(always)]
    fn find(&self, address: usize, len: usize) -> Option<&Entry> {
        if self.status != SeedStatus::Ready
            || len == 0
            || len > MAX_BYTES
            || self.lengths & (1_u64 << len) == 0
            || self.entries.is_empty()
        {
            return None;
        }
        let (word, bit) = Self::front_filter_bit(address, len);
        if self.front_filter[word] & bit == 0 {
            return None;
        }
        self.find_possible(address, len)
    }

    // Keep the exact probe loop out of every monomorphized scalar reader,
    // while allowing non-country reads to reject without a cross-crate call.
    #[inline(never)]
    fn find_possible(&self, address: usize, len: usize) -> Option<&Entry> {
        // Possible front-filter hits retain the original mixed-hash filter
        // and full exact address/length comparison, including probe bounds.
        let hash = Self::hash(address, len);
        let (word, bit) = Self::filter_bit(hash);
        if self.filter[word] & bit == 0 {
            return None;
        }
        let first = hash & (self.entries.len() - 1);
        for probe in 0..MAX_PROBES.min(self.entries.len()) {
            let entry = &self.entries[(first + probe) & (self.entries.len() - 1)];
            if entry.len == 0 {
                return None;
            }
            if entry.address == address && entry.len == len {
                return Some(entry);
            }
        }
        None
    }

    fn observe(&mut self, address: usize, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.observed_reads = self.observed_reads.saturating_add(1);
        if self.status != SeedStatus::Ready {
            return;
        }
        if address == 0 || address.checked_add(bytes.len()).is_none() || bytes.len() > MAX_BYTES {
            self.status = SeedStatus::Unavailable {
                reason: "country authentication read is outside cache bounds",
            };
            return;
        }
        if !self.entries.is_empty() {
            let hash = Self::hash(address, bytes.len());
            let first = hash & (self.entries.len() - 1);
            let mask = self.entries.len() - 1;
            for probe in 0..MAX_PROBES.min(self.entries.len()) {
                let entry = &mut self.entries[(first + probe) & mask];
                if entry.len == 0 {
                    entry.address = address;
                    entry.len = bytes.len();
                    entry.bytes[..bytes.len()].copy_from_slice(bytes);
                    let (word, bit) = Self::filter_bit(hash);
                    self.filter[word] |= bit;
                    let (word, bit) = Self::front_filter_bit(address, bytes.len());
                    self.front_filter[word] |= bit;
                    self.lengths |= 1_u64 << bytes.len();
                    self.len += 1;
                    return;
                }
                if entry.address == address && entry.len == bytes.len() {
                    if entry.bytes[..entry.len] != *bytes {
                        self.status = SeedStatus::Unavailable {
                            reason: "country authentication changed during census",
                        };
                    }
                    return;
                }
            }
        }
        self.status = SeedStatus::Capacity { required_cells: 0 };
    }
}

/// This type is private so callers cannot accidentally seed arbitrary fields.
struct RecordingMemory<'a, M> {
    source: &'a M,
    cache: RefCell<&'a mut Cache>,
}

impl<M: Memory> Memory for RecordingMemory<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let success = self.source.copy(address, destination);
        let mut cache = self.cache.borrow_mut();
        if success {
            cache.observe(address, destination);
        } else {
            cache.status = SeedStatus::Unavailable {
                reason: "country authentication source unreadable during census",
            };
        }
        success
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.source.copy_scattered(addresses, destinations);
        let mut cache = self.cache.borrow_mut();
        for index in 0..copied {
            cache.observe(addresses[index], destinations[index]);
        }
        if copied < N {
            cache.status = SeedStatus::Unavailable {
                reason: "country authentication source unreadable during census",
            };
        }
        copied
    }
}

/// A single preparation's read-only view, borrowed until its final verification.
pub struct CountryMemory<'a, M> {
    source: &'a M,
    cache: &'a Cache,
}

impl<M: Memory> CountryMemory<'_, M> {
    pub fn seed_status(&self) -> SeedStatus {
        self.cache.status
    }

    pub fn cached_entries(&self) -> usize {
        if self.cache.status == SeedStatus::Ready {
            self.cache.len
        } else {
            0
        }
    }

    /// Consume this view after table walks and re-read every cached exact span
    /// directly. No cached result or country-pointer observation survives this
    /// check into the next preparation. A failure invalidates dependent tables.
    pub fn verify(self) -> Result<Verification, &'static str> {
        if self.cache.status != SeedStatus::Ready {
            return Ok(Verification::NotCached);
        }
        let mut entries = self.cache.entries.iter().filter(|entry| entry.len != 0);
        let mut bytes = [[0_u8; MAX_BYTES]; VERIFY_BATCH];
        loop {
            let batch: [Option<&Entry>; VERIFY_BATCH] = std::array::from_fn(|_| entries.next());
            let len = batch.iter().take_while(|entry| entry.is_some()).count();
            if len == 0 {
                return Ok(Verification::Matched {
                    entries: self.cache.len,
                });
            }
            let addresses = batch.map(|entry| entry.map_or(0, |entry| entry.address));
            let mut fields = bytes.iter_mut().enumerate();
            let copied = self.source.copy_scattered(
                addresses,
                &mut std::array::from_fn(|_| {
                    let (index, field) = fields.next().unwrap();
                    &mut field[..batch[index].map_or(0, |entry| entry.len)]
                }),
            );
            for index in 0..copied.min(len) {
                let entry = batch[index].unwrap();
                if bytes[index][..entry.len] != entry.bytes[..entry.len] {
                    return Err("country authentication changed during table walks");
                }
            }
            if copied < len {
                return Err("country authentication became unreadable during table walks");
            }
        }
    }
}

impl<M: Memory> Memory for CountryMemory<'_, M> {
    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        (self.cache.status == SeedStatus::Ready
            && self.cache.roster_len != 0
            && base == self.cache.roster_base
            && game_state == self.cache.roster_game_state)
            .then(|| {
                ccountry::AuthenticatedRoster::new(&self.cache.roster[..self.cache.roster_len])
            })
    }

    #[inline(always)]
    fn copy_records(
        &self,
        sources: crate::memory::RecordSources<'_>,
        layout: &crate::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        // Fixed-layout gathers intentionally remain fresh source reads. This
        // preserves their single guarded batch; the separately seeded country
        // authentication observations are still verified after table walks.
        self.source.copy_records(sources, layout, destination)
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if let Some(entry) = self.cache.find(address, destination.len()) {
            destination.copy_from_slice(&entry.bytes[..entry.len]);
            true
        } else {
            self.source.copy(address, destination)
        }
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if self.cache.status != SeedStatus::Ready {
            return self.source.copy_scattered(addresses, destinations);
        }
        let hits: [Option<&Entry>; N] = std::array::from_fn(|index| {
            self.cache.find(addresses[index], destinations[index].len())
        });
        if hits.iter().all(Option::is_none) {
            // Common non-country batches keep exactly one original guarded
            // scatter call, with the original destinations and failure prefix.
            return self.source.copy_scattered(addresses, destinations);
        }
        let copied = if destinations
            .iter()
            .zip(hits)
            .all(|(destination, hit)| destination.is_empty() || hit.is_some())
        {
            N
        } else {
            let mut fields = destinations.iter_mut().enumerate();
            self.source.copy_scattered(
                addresses,
                &mut std::array::from_fn(|_| {
                    let (index, destination) = fields.next().unwrap();
                    if hits[index].is_some() {
                        &mut []
                    } else {
                        &mut **destination
                    }
                }),
            )
        };
        // Hits after a failed miss remain untouched, just as later native spans
        // do. A partially copied failed miss is never counted as complete.
        for index in 0..copied {
            if let Some(entry) = hits[index] {
                destinations[index].copy_from_slice(&entry.bytes[..entry.len]);
            }
        }
        copied
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::{cgame_state, read, walk::Outcome};

    #[test]
    fn shared_snapshot_refuses_reseed_while_leased_without_reading_or_mutating() {
        let source = Synthetic::countries();
        let mut owner = Arc::new(Cache::with_capacity(64));
        let first = Cache::prepare_shared(&mut owner, &source, 0, 0x1000).unwrap();
        let initial = shared_walks(&first.view(&source));
        source.observations.borrow_mut().clear();
        let worker = Arc::clone(&first);
        assert_eq!(
            Cache::prepare_shared(&mut owner, &source, 0, 0x1000).err(),
            Some("country snapshot still leased by a reader")
        );
        assert!(source.observations.borrow().is_empty());
        assert_eq!(shared_walks(&worker.view(&source)), initial);
        assert!(worker.view(&source).verify().is_ok());
        drop(worker);
        drop(first);
        let next = Cache::prepare_shared(&mut owner, &source, 0, 0x1000).unwrap();
        assert_eq!(shared_walks(&next.view(&source)), initial);
    }

    #[test]
    fn shared_snapshot_views_do_not_reseed_and_outer_verification_rejects_mutation() {
        let source = Synthetic::countries();
        let mut owner = Arc::new(Cache::with_capacity(64));
        let snapshot = Cache::prepare_shared(&mut owner, &source, 0, 0x1000).unwrap();
        source.observations.borrow_mut().clear();
        let first = snapshot.view(&source);
        let second = snapshot.view(&source);
        assert_eq!(
            ccountry::Reader::new(&first, 0, 0x1000).unwrap().resolve(7),
            ccountry::Reader::new(&second, 0, 0x1000)
                .unwrap()
                .resolve(7)
        );
        assert!(source.observations.borrow().is_empty());
        source.put(0x2000 + ccountry::ID, &2_u32.to_le_bytes());
        assert!(snapshot.view(&source).verify().is_err());
        source.fault.set(Some(0x2000 + ccountry::ID));
        assert!(snapshot.view(&source).verify().is_err());
    }

    #[test]
    fn front_filter_keeps_every_inserted_length_and_address_class() {
        let mut cache = Cache::with_capacity(4096);
        cache.status = SeedStatus::Ready;
        let bytes = std::array::from_fn::<_, MAX_BYTES, _>(|i| i as u8);
        for address in [
            1,
            7,
            8,
            31,
            4095,
            4096,
            0x1234_5678,
            0x0000_7fff_ffff_fff0,
            0xffff_8000_0000_0001,
            usize::MAX - MAX_BYTES,
        ] {
            for len in 1..=MAX_BYTES {
                cache.observe(address, &bytes[..len]);
                assert_eq!(cache.status, SeedStatus::Ready);
                let (word, bit) = Cache::front_filter_bit(address, len);
                assert_ne!(cache.front_filter[word] & bit, 0);
                let entry = cache.find(address, len).expect("inserted key rejected");
                assert_eq!(entry.address, address);
                assert_eq!(entry.len, len);
                assert_eq!(&entry.bytes[..len], &bytes[..len]);
            }
        }
        for entry in cache.entries.iter().filter(|entry| entry.len != 0) {
            assert!(cache.find(entry.address, entry.len).is_some());
        }
        assert!(cache.find(1, 0).is_none());
        assert!(cache.find(1, MAX_BYTES + 1).is_none());
    }

    #[test]
    fn front_filter_collision_never_serves_wrong_bytes_or_masks_source_failure() {
        let source = Synthetic::countries();
        let mut cache = Cache::with_capacity(64);
        cache.status = SeedStatus::Ready;
        let bytes = (ccountry::VTABLE_RVA as u64).to_le_bytes();
        cache.observe(0x2000, &bytes);
        let key = Cache::front_filter_bit(0x2000, 8);
        let collision = (1..=FILTER_BITS * 8)
            .find(|&address| address != 0x2000 && Cache::front_filter_bit(address, 8) == key)
            .expect("finite bitmap permits collisions");
        assert!(cache.find(collision, 8).is_none());
        source.fault.set(Some(collision));
        let memory = CountryMemory {
            source: &source,
            cache: &cache,
        };
        let mut output = [0; 8];
        assert!(!memory.copy(collision, &mut output));
        assert_eq!(source.observations.borrow().last(), Some(&(collision, 8)));
        assert_eq!(memory.verify(), Ok(Verification::Matched { entries: 1 }));

        let mut changed = bytes;
        changed[0] ^= 1;
        cache.observe(0x2000, &changed);
        assert!(matches!(cache.status, SeedStatus::Unavailable { .. }));
        assert!(cache.find(0x2000, 8).is_none());
    }

    #[test]
    fn front_filter_rebuilds_only_successful_seed_keys_and_has_fixed_accounting() {
        let source = Synthetic::countries();
        let mut cache = Cache::with_capacity(64);
        let allocated = cache.allocated_bytes();
        assert_eq!(std::mem::size_of_val(&cache.front_filter), 2048);
        let expected_allocated = std::mem::size_of_val(cache.entries.as_ref())
            + std::mem::size_of_val(&cache.filter)
            + std::mem::size_of_val(&cache.front_filter)
            + std::mem::size_of_val(&cache.lengths)
            + std::mem::size_of_val(cache.roster.as_ref())
            + std::mem::size_of_val(&cache.roster)
            + std::mem::size_of_val(&cache.roster_len)
            + std::mem::size_of_val(&cache.roster_base)
            + std::mem::size_of_val(&cache.roster_game_state);
        assert_eq!(allocated, expected_allocated);
        for _ in 0..2 {
            let memory = cache.prepare(&source, 0, 0x1000);
            assert_eq!(memory.seed_status(), SeedStatus::Ready);
            assert!(memory.verify().is_ok());
            let mut expected = [0; FILTER_WORDS];
            for entry in cache.entries.iter().filter(|entry| entry.len != 0) {
                let (word, bit) = Cache::front_filter_bit(entry.address, entry.len);
                expected[word] |= bit;
                assert!(cache.find(entry.address, entry.len).is_some());
            }
            assert_eq!(cache.front_filter, expected);
            assert!(expected.iter().any(|&word| word != 0));
            assert_eq!(cache.allocated_bytes(), allocated);
        }
        source.fault.set(Some(0x1000 + cgame_state::COUNTRIES));
        let memory = cache.prepare(&source, 0, 0x1000);
        assert!(matches!(
            memory.seed_status(),
            SeedStatus::Unavailable { .. }
        ));
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
        assert_eq!(cache.front_filter, [0; FILTER_WORDS]);
        assert_eq!(cache.allocated_bytes(), allocated);

        let mut disabled = Cache::with_capacity(0);
        disabled.status = SeedStatus::Ready;
        disabled.observe(0x2000, &[1]);
        assert!(matches!(disabled.status, SeedStatus::Capacity { .. }));
        assert_eq!(disabled.front_filter, [0; FILTER_WORDS]);
    }

    struct Synthetic {
        bytes: RefCell<Vec<u8>>,
        fault: Cell<Option<usize>>,
        guards: Cell<usize>,
        id_reads: Cell<usize>,
        change_id_during_seed: Cell<bool>,
        observations: RefCell<Vec<(usize, usize)>>,
    }

    impl Synthetic {
        fn countries() -> Self {
            let memory = Self {
                bytes: RefCell::new(vec![0; 0x3000]),
                fault: Cell::new(None),
                guards: Cell::new(0),
                id_reads: Cell::new(0),
                change_id_during_seed: Cell::new(false),
                observations: RefCell::new(Vec::new()),
            };
            for (offset, pointer, count) in [
                (cgame_state::COUNTRIES, 0x1500_u64, 4_u32),
                (cgame_state::COUNTRY_LINKS, 0x1700, 8),
            ] {
                memory.put(0x1000 + offset, &pointer.to_le_bytes());
                memory.put(0x1000 + offset + 8, &count.to_le_bytes());
                memory.put(0x1000 + offset + 12, &count.to_le_bytes());
            }
            // Slots 1 and 2 alias one object with native id 7; slot 3 is null.
            for slot in [1, 2] {
                memory.put(0x1500 + slot * 8, &0x2000_u64.to_le_bytes());
            }
            for link in [1, 2, 7] {
                memory.put(0x1700 + link * 4, &1_u32.to_le_bytes());
            }
            memory.put(0x2000, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
            memory.put(0x2000 + ccountry::ID, &7_u32.to_le_bytes());
            memory.put(0x2000 + ccountry::COMMAND_POWER, &11_i64.to_le_bytes());
            memory
        }

        fn put(&self, address: usize, value: &[u8]) {
            let start = address - 0x1000;
            self.bytes.borrow_mut()[start..start + value.len()].copy_from_slice(value);
        }

        fn copy_source(&self, address: usize, destination: &mut [u8]) -> bool {
            self.observations
                .borrow_mut()
                .push((address, destination.len()));
            if self
                .fault
                .get()
                .is_some_and(|fault| (address..address + destination.len()).contains(&fault))
            {
                destination[0] = 0x7e;
                return false;
            }
            if address == 0x2000 + ccountry::ID && destination.len() == 4 {
                self.id_reads.set(self.id_reads.get() + 1);
                if self.change_id_during_seed.get() && self.id_reads.get() == 2 {
                    self.put(address, &2_u32.to_le_bytes());
                }
            }
            crate::memory::SliceMemory {
                base: 0x1000,
                bytes: &self.bytes.borrow(),
            }
            .copy(address, destination)
        }
    }

    impl Memory for Synthetic {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.guards.set(self.guards.get() + 1);
            self.copy_source(address, destination)
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.guards.set(self.guards.get() + 1);
            for index in 0..N {
                if destinations[index].is_empty() {
                    continue;
                }
                if addresses[index] == 0
                    || addresses[index]
                        .checked_add(destinations[index].len())
                        .is_none()
                    || !self.copy_source(addresses[index], destinations[index])
                {
                    return index;
                }
            }
            N
        }
    }

    fn shared_walks(memory: &impl Memory) -> (Vec<ccountry::Country>, Vec<ccountry::LinkRow>) {
        let reader = ccountry::Reader::new(memory, 0, 0x1000).unwrap();
        let countries = (1..reader.count())
            .filter_map(|slot| reader.at(slot).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(reader.resolve(7).unwrap(), countries[0]);
        let mut links = [ccountry::LinkRow::default(); 8];
        let Outcome::Rows { len } = ccountry::walk_links(memory, 0, 0x1000, &mut links) else {
            panic!("the synthetic aliases must remain readable");
        };
        (countries, links[..len].to_vec())
    }

    #[test]
    fn negative_filter_collisions_and_saturation_still_require_exact_keys() {
        // Pigeonhole-bounded search: page-aligned addresses exercise the same
        // existing full-address hash; two keys must share one filter bit.
        let mut seen = vec![None; FILTER_BITS];
        let (first, second) = (0..=FILTER_BITS)
            .find_map(|index| {
                let address = 0x1000 + index * 0x1000;
                let bit = Cache::hash(address, 8) & (FILTER_BITS - 1);
                seen[bit]
                    .replace(address)
                    .map(|previous| (previous, address))
            })
            .expect("more distinct keys than filter bits must collide");
        let mut cache = Cache::with_capacity(64);
        cache.status = SeedStatus::Ready;
        cache.observe(first, &[0x41; 8]);
        assert_eq!(
            Cache::filter_bit(Cache::hash(first, 8)),
            Cache::filter_bit(Cache::hash(second, 8))
        );
        assert!(cache.find(second, 8).is_none());
        cache.observe(second, &[0x72; 8]);
        cache.observe(first, &[0x41; 8]);
        assert_eq!(cache.len, 2);
        assert_eq!(cache.lengths, 1 << 8);
        assert_eq!(&cache.find(first, 8).unwrap().bytes[..8], &[0x41; 8]);
        assert_eq!(&cache.find(second, 8).unwrap().bytes[..8], &[0x72; 8]);

        // Even a saturated filter may only increase exact-lookup work. It
        // cannot serve an overlapping key, a different width, or an empty span.
        cache.filter.fill(u64::MAX);
        cache.lengths = u64::MAX;
        for length in [0, 1, 4, 7, 8, 16, 32, 33, 64, usize::MAX] {
            assert_eq!(cache.find(first, length).is_some(), length == 8);
            assert!(cache.find(first + 4, length).is_none());
        }
        cache.status = SeedStatus::Capacity {
            required_cells: 128,
        };
        assert!(cache.find(first, 8).is_none());
        cache.status = SeedStatus::Unavailable { reason: "fixture" };
        assert!(cache.find(first, 8).is_none());
    }

    #[test]
    fn negative_filter_learns_every_supported_width_and_reports_owned_storage() {
        let mut cache = Cache::with_capacity(256);
        cache.status = SeedStatus::Ready;
        for length in 1..=MAX_BYTES {
            cache.observe(0x1000 + length * 64, &[0x5a; MAX_BYTES][..length]);
        }
        assert_eq!(cache.status, SeedStatus::Ready);
        assert_eq!(cache.lengths, (1_u64 << (MAX_BYTES + 1)) - 2);
        for length in 1..=MAX_BYTES {
            let entry = cache.find(0x1000 + length * 64, length).unwrap();
            assert_eq!(&entry.bytes[..length], &[0x5a; MAX_BYTES][..length]);
        }
        assert_eq!(
            cache.allocated_bytes(),
            256 * (std::mem::size_of::<Entry>() + std::mem::size_of::<Option<ccountry::Country>>())
                + 2 * std::mem::size_of::<[u64; FILTER_WORDS]>()
                + std::mem::size_of::<u64>()
                + std::mem::size_of::<Box<[Option<ccountry::Country>]>>()
                + 3 * std::mem::size_of::<usize>()
        );
        assert_eq!(
            Cache::with_capacity(0).allocated_bytes(),
            2 * std::mem::size_of::<[u64; FILTER_WORDS]>()
                + std::mem::size_of::<u64>()
                + std::mem::size_of::<Box<[Option<ccountry::Country>]>>()
                + 3 * std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn preparations_replace_all_negative_filter_bits_and_old_keys() {
        let source = Synthetic::countries();
        let mut cache = Cache::with_capacity(64);
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(memory.seed_status(), SeedStatus::Ready);
        assert!(memory.verify().is_ok());
        cache.filter.fill(u64::MAX);
        cache.lengths = u64::MAX;
        for slot in [1, 2] {
            source.put(0x1500 + slot * 8, &0x2800_u64.to_le_bytes());
        }
        source.put(0x2800, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
        source.put(0x2808, &7_u32.to_le_bytes());
        source.put(0x2008, &99_u32.to_le_bytes());
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(memory.seed_status(), SeedStatus::Ready);
        assert_eq!(memory.cache.lengths, (1 << 4) | (1 << 8) | (1 << 16));
        let mut expected = [0; FILTER_WORDS];
        for entry in memory.cache.entries.iter().filter(|entry| entry.len != 0) {
            let (word, bit) = Cache::filter_bit(Cache::hash(entry.address, entry.len));
            expected[word] |= bit;
            assert!(memory.cache.find(entry.address, entry.len).is_some());
        }
        assert_eq!(memory.cache.filter, expected);
        assert!(memory.cache.find(0x2008, 4).is_none());
        let before = source.guards.get();
        assert_eq!(read::<u32>(&memory, 0x2008), Some(99));
        assert_eq!(source.guards.get(), before + 1);
        assert_eq!(read::<u32>(&memory, 0x2808), Some(7));
        assert_eq!(source.guards.get(), before + 1);
        assert!(memory.verify().is_ok());

        // An immediate failed census cannot retain bits from the previous one.
        source.fault.set(Some(0x1000 + cgame_state::COUNTRIES));
        let memory = cache.prepare(&source, 0, 0x1000);
        assert!(matches!(
            memory.seed_status(),
            SeedStatus::Unavailable { .. }
        ));
        assert_eq!(memory.cache.filter, [0; FILTER_WORDS]);
        assert_eq!(memory.cache.lengths, 0);
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
    }

    #[test]
    fn repeated_walkers_share_authentication_but_scalars_and_unseeded_links_stay_live() {
        let source = Synthetic::countries();
        let expected = shared_walks(&source);
        let direct_cost = source.guards.replace(0);
        let mut cache = Cache::with_capacity(64);
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(memory.seed_status(), SeedStatus::Ready);
        assert_eq!(memory.cached_entries(), 10);
        let seed_cost = source.guards.get();
        for _ in 0..32 {
            assert_eq!(shared_walks(&memory), expected);
        }
        // Only four unused zero link slots were absent from Reader's seed
        // census. They stay direct reads in the independent links walker.
        assert_eq!(source.guards.get() - seed_cost, 4 * 32);
        assert!(source.guards.get() < direct_cost * 32);
        let scalar = i64::MIN + 987;
        source.put(0x2000 + ccountry::COMMAND_POWER, &scalar.to_le_bytes());
        assert_eq!(
            read::<i64>(&memory, 0x2000 + ccountry::COMMAND_POWER),
            Some(scalar)
        );
        // A wider or overlapping read cannot be assembled from cached pieces.
        source.put(0x2000 + ccountry::ID_INSTANCE, &u32::MAX.to_le_bytes());
        assert_eq!(read::<u64>(&memory, 0x2008), Some(0xffff_ffff_0000_0007));
        let before_verify = source.guards.get();
        assert_eq!(memory.verify(), Ok(Verification::Matched { entries: 10 }));
        assert_eq!(source.guards.get(), before_verify + 1);
    }

    #[test]
    fn scatter_misses_keep_one_guard_and_hits_after_a_fault_stay_untouched() {
        let source = Synthetic::countries();
        let mut cache = Cache::with_capacity(64);
        let memory = cache.prepare(&source, 0, 0x1000);
        let scalar = 0x2000 + ccountry::COMMAND_POWER;
        source.fault.set(Some(scalar + 2));
        for addresses in [[0x1800, scalar, 0x1810], [0x2008, scalar, 0x2000]] {
            let mut first = [0xa5; 4];
            let mut failed = [0xa5; 8];
            let mut later = [0xa5; 8];
            let before = source.guards.get();
            source.observations.borrow_mut().clear();
            assert_eq!(
                memory.copy_scattered(addresses, &mut [&mut first, &mut failed, &mut later]),
                1
            );
            assert_eq!(source.guards.get(), before + 1);
            assert_eq!(
                *source.observations.borrow(),
                if addresses[0] == 0x2008 {
                    vec![(scalar, 8)]
                } else {
                    vec![(0x1800, 4), (scalar, 8)]
                }
            );
            assert_eq!(
                first,
                if addresses[0] == 0x2008 {
                    [7, 0, 0, 0]
                } else {
                    [0; 4]
                }
            );
            assert_eq!(failed, [0x7e, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]);
            assert_eq!(later, [0xa5; 8]);
        }
        let mut failed = [0xa5; 8];
        let mut later_hit = [0xa5; 4];
        source.observations.borrow_mut().clear();
        assert_eq!(
            memory.copy_scattered([scalar, 0x2008], &mut [&mut failed, &mut later_hit]),
            0
        );
        assert_eq!(later_hit, [0xa5; 4]);
        assert_eq!(*source.observations.borrow(), vec![(scalar, 8)]);
        source.fault.set(None);
        let mut empty = [];
        let mut id = [0; 4];
        let before = source.guards.get();
        source.observations.borrow_mut().clear();
        assert_eq!(
            memory.copy_scattered([0, 0x2008], &mut [&mut empty, &mut id]),
            2
        );
        assert_eq!(source.guards.get(), before);
        assert!(source.observations.borrow().is_empty());
        assert_eq!(id, [7, 0, 0, 0]);
        assert!(memory.verify().is_ok());
    }

    #[test]
    fn direct_verification_rejects_mutated_or_unreadable_cached_sources() {
        for address in [
            0x1000 + cgame_state::COUNTRIES,
            0x1508,
            0x1704,
            0x2000,
            0x2008,
        ] {
            let source = Synthetic::countries();
            let mut cache = Cache::with_capacity(64);
            let memory = cache.prepare(&source, 0, 0x1000);
            source.bytes.borrow_mut()[address - 0x1000] ^= 1;
            assert!(memory.authenticated_countries(0, 0x1000).is_some());
            assert_eq!(
                ccountry::Reader::new(&memory, 0, 0x1000).unwrap().at(1),
                Ok(Some(ccountry::Country {
                    pointer: 0x2000,
                    canonical_slot: 1,
                    id: 7
                })),
            );
            assert_eq!(
                memory.verify(),
                Err("country authentication changed during table walks")
            );
            let source = Synthetic::countries();
            let memory = cache.prepare(&source, 0, 0x1000);
            source.fault.set(Some(address));
            assert!(memory.authenticated_countries(0, 0x1000).is_some());
            assert_eq!(
                ccountry::Reader::new(&memory, 0, 0x1000).unwrap().at(1),
                Ok(Some(ccountry::Country {
                    pointer: 0x2000,
                    canonical_slot: 1,
                    id: 7
                })),
            );
            assert_eq!(
                memory.verify(),
                Err("country authentication became unreadable during table walks")
            );
        }
    }

    #[test]
    fn failed_censuses_and_capacity_never_serve_a_cached_prefix() {
        for capacity in [0, 1] {
            let source = Synthetic::countries();
            let mut cache = Cache::with_capacity(capacity);
            let memory = cache.prepare(&source, 0, 0x1000);
            assert!(
                matches!(memory.seed_status(), SeedStatus::Capacity { required_cells } if required_cells > capacity)
            );
            assert_eq!(memory.cached_entries(), 0);
            source.put(0x2008, &2_u32.to_le_bytes());
            assert_eq!(read::<u32>(&memory, 0x2008), Some(2));
            source.fault.set(Some(0x2008));
            let mut failed = [0xa5; 4];
            assert!(!memory.copy(0x2008, &mut failed));
            assert_eq!(failed, [0x7e, 0xa5, 0xa5, 0xa5]);
            source.fault.set(None);
            assert_eq!(memory.verify(), Ok(Verification::NotCached));
            assert_eq!(cache.capacity(), capacity);
        }
        let source = Synthetic::countries();
        source.fault.set(Some(0x2008));
        let mut cache = Cache::with_capacity(64);
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(
            memory.seed_status(),
            SeedStatus::Unavailable {
                reason: "country id unreadable"
            }
        );
        assert_eq!(memory.cached_entries(), 0);
        let mut failed = [0xa5; 4];
        assert!(!memory.copy(0x2008, &mut failed));
        assert_eq!(failed, [0x7e, 0xa5, 0xa5, 0xa5]);
        source.fault.set(None);
        source.put(0x2008, &2_u32.to_le_bytes());
        assert_eq!(read::<u32>(&memory, 0x2008), Some(2));
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
        // The next preparation forgets both a failed prefix and all old ids.
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(memory.seed_status(), SeedStatus::Ready);
        assert_eq!(read::<u32>(&memory, 0x2008), Some(2));
        assert!(memory.verify().is_ok());
        let memory = cache.prepare(&source, 0, usize::MAX);
        assert!(matches!(
            memory.seed_status(),
            SeedStatus::Unavailable { .. }
        ));
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
        source.put(0x2000, &0_u64.to_le_bytes());
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(
            memory.seed_status(),
            SeedStatus::Unavailable {
                reason: "country vtable mismatch"
            }
        );
        source.put(0x1508, &0_u64.to_le_bytes());
        assert_eq!(read::<u64>(&memory, 0x1508), Some(0));
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
    }

    #[test]
    fn individually_valid_aliases_that_change_during_seeding_disable_the_cache() {
        let source = Synthetic::countries();
        source.change_id_during_seed.set(true);
        let mut cache = Cache::with_capacity(64);
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(
            memory.seed_status(),
            SeedStatus::Unavailable {
                reason: "country authentication changed during census"
            }
        );
        assert!(memory.authenticated_countries(0, 0x1000).is_none());
        assert_eq!(read::<u32>(&memory, 0x2008), Some(2));
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
    }

    /// The byte-cache-only reference deliberately does not forward the new
    /// capability. It otherwise keeps the original source and batch semantics.
    struct ByteOnly<'a, M>(&'a M);

    impl<M: Memory> Memory for ByteOnly<'_, M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.0.copy(address, destination)
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.0.copy_scattered(addresses, destinations)
        }

        fn copy_records(
            &self,
            sources: crate::memory::RecordSources<'_>,
            layout: &crate::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.0.copy_records(sources, layout, destination)
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CountryResults {
        slots: Vec<Result<Option<ccountry::Country>, &'static str>>,
        resolved: Vec<Result<ccountry::Country, &'static str>>,
        links: Vec<Result<Option<u32>, &'static str>>,
    }

    fn country_results(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> Result<CountryResults, &'static str> {
        let reader = ccountry::Reader::new(memory, base, game_state)?;
        let queries = (0..=8).chain(std::iter::once(u32::MAX));
        Ok(CountryResults {
            slots: queries.clone().map(|slot| reader.at(slot)).collect(),
            resolved: queries.clone().map(|raw| reader.resolve(raw)).collect(),
            links: queries.map(|raw| reader.link(raw)).collect(),
        })
    }

    #[test]
    fn authenticated_roster_seed_keeps_original_alias_and_fault_read_order() {
        let spans = [
            (0x1000 + cgame_state::COUNTRIES, 16),
            (0x1000 + cgame_state::COUNTRY_LINKS, 16),
            (0x1508, 8),
            (0x1510, 8),
            (0x1518, 8),
            (0x2000, 8),
            (0x2008, 4),
            (0x1704, 4),
            (0x1708, 4),
            (0x171c, 4),
        ];
        for fault in std::iter::once(None).chain(
            spans
                .into_iter()
                .flat_map(|(address, len)| [Some(address), Some(address + len - 1)]),
        ) {
            let original = Synthetic::countries();
            let actual = Synthetic::countries();
            original.fault.set(fault);
            actual.fault.set(fault);
            let old_census = (|| {
                let reader = ccountry::Reader::new(&original, 0, 0x1000)?;
                for slot in 1..reader.count() {
                    reader.at(slot)?;
                }
                Ok::<(), &'static str>(())
            })();
            let mut cache = Cache::with_capacity(64);
            let memory = cache.prepare(&actual, 0, 0x1000);
            assert_eq!(
                *actual.observations.borrow(),
                *original.observations.borrow(),
                "{fault:?}"
            );
            match old_census {
                Ok(()) => assert!(memory.authenticated_countries(0, 0x1000).is_some()),
                Err(reason) => {
                    assert_eq!(memory.seed_status(), SeedStatus::Unavailable { reason });
                    assert!(memory.authenticated_countries(0, 0x1000).is_none());
                }
            }
        }
    }

    #[test]
    fn authenticated_roster_matches_byte_cache_faults_and_verification() {
        let faults = [
            None,
            Some(0x1000 + cgame_state::COUNTRIES),
            Some(0x1000 + cgame_state::COUNTRY_LINKS + 15),
            Some(0x1508),
            Some(0x1517),
            Some(0x1518),
            Some(0x2000),
            Some(0x200b),
            Some(0x1704),
            Some(0x1708),
            Some(0x171f),
            Some(0x1710),
            Some(0x2000 + ccountry::COMMAND_POWER),
        ];
        for capacity in [0, 1, 64] {
            for before_seed in [false, true] {
                for fault in faults {
                    let actual = Synthetic::countries();
                    let original = Synthetic::countries();
                    if before_seed {
                        actual.fault.set(fault);
                        original.fault.set(fault);
                    }
                    let mut actual_cache = Cache::with_capacity(capacity);
                    let mut original_cache = Cache::with_capacity(capacity);
                    let actual_memory = actual_cache.prepare(&actual, 0, 0x1000);
                    let original_memory = original_cache.prepare(&original, 0, 0x1000);
                    assert_eq!(actual_memory.seed_status(), original_memory.seed_status());
                    assert_eq!(
                        actual_memory.cached_entries(),
                        original_memory.cached_entries()
                    );
                    if !before_seed {
                        actual.fault.set(fault);
                        original.fault.set(fault);
                    }
                    assert_eq!(
                        country_results(&actual_memory, 0, 0x1000),
                        country_results(&ByteOnly(&original_memory), 0, 0x1000),
                        "capacity={capacity} before_seed={before_seed} fault={fault:?}",
                    );
                    assert_eq!(
                        read::<i64>(&actual_memory, 0x2000 + ccountry::COMMAND_POWER),
                        read::<i64>(
                            &ByteOnly(&original_memory),
                            0x2000 + ccountry::COMMAND_POWER
                        ),
                    );
                    assert_eq!(
                        *actual.observations.borrow(),
                        *original.observations.borrow()
                    );
                    assert_eq!(actual_memory.verify(), original_memory.verify());
                    assert_eq!(
                        *actual.observations.borrow(),
                        *original.observations.borrow()
                    );
                }
            }
        }
    }

    #[test]
    fn authenticated_roster_keeps_arbitrary_link_reads_and_context_identity() {
        let source = Synthetic::countries();
        let mut cache = Cache::with_capacity(64);
        let memory = cache.prepare(&source, 0, 0x1000);
        assert!(memory.authenticated_countries(0, 0x1000).is_some());
        assert!(memory.authenticated_countries(1, 0x1000).is_none());
        assert!(memory.authenticated_countries(0, 0x1100).is_none());
        for (base, state) in [(1, 0x1000), (0, 0x1100), (usize::MAX, 0x1000)] {
            assert_eq!(
                country_results(&memory, base, state),
                country_results(&ByteOnly(&memory), base, state)
            );
        }
        let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
        let country = ccountry::Country {
            pointer: 0x2000,
            canonical_slot: 1,
            id: 7,
        };
        assert_eq!(reader.at(1), Ok(Some(country)));
        assert_eq!(reader.at(2), Ok(None));
        assert_eq!(reader.at(3), Ok(None));
        assert_eq!(reader.at(0), Err("country id is zero"));
        for (slot, expected) in [
            (1_u32, Ok(country)),
            (
                2,
                Err("country link does not resolve to its canonical object"),
            ),
            (3, Err("country link resolves to null country")),
            (0, Err("country link unreadable or outside country table")),
            (4, Err("country link unreadable or outside country table")),
            (1, Ok(country)),
        ] {
            // Raw id 4 was never seeded; each requested observation stays live.
            source.put(0x1710, &slot.to_le_bytes());
            source.observations.borrow_mut().clear();
            assert_eq!(reader.resolve(4), expected);
            assert_eq!(&*source.observations.borrow(), &[(0x1710, 4)]);
        }
        source.fault.set(Some(0x1710));
        source.observations.borrow_mut().clear();
        assert_eq!(
            reader.resolve(4),
            Err("country link unreadable or outside country table")
        );
        assert_eq!(&*source.observations.borrow(), &[(0x1710, 4)]);
        source.fault.set(None);
        // An uncached link is not silently added to final verification.
        assert_eq!(memory.verify(), Ok(Verification::Matched { entries: 10 }));
    }

    #[test]
    fn authenticated_roster_reprepare_replaces_aliases_nulls_and_short_storage() {
        let source = Synthetic::countries();
        let mut cache = Cache::with_capacity(64);
        let allocated = cache.allocated_bytes();
        let memory = cache.prepare(&source, 0, 0x1000);
        assert!(memory.authenticated_countries(0, 0x1000).is_some());
        assert!(memory.verify().is_ok());

        // A new object/id with one now-null alias cannot retain the old object.
        source.put(0x1508, &0x2800_u64.to_le_bytes());
        source.put(0x1510, &0_u64.to_le_bytes());
        source.put(0x2800, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
        source.put(0x2808, &1_u32.to_le_bytes());
        let memory = cache.prepare(&source, 0, 0x1000);
        let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
        assert_eq!(
            reader.at(1),
            Ok(Some(ccountry::Country {
                pointer: 0x2800,
                canonical_slot: 1,
                id: 1
            }))
        );
        assert_eq!(reader.at(2), Ok(None));
        assert_eq!(reader.resolve(2), reader.resolve(1));
        assert!(memory.verify().is_ok());

        source.put(0x1000 + cgame_state::COUNTRIES + 12, &2_u32.to_le_bytes());
        let memory = cache.prepare(&source, 0, 0x1000);
        assert!(memory.authenticated_countries(0, 0x1000).is_some());
        let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
        assert_eq!(reader.count(), 2);
        assert_eq!(
            reader.at(2),
            Err("country pointer unreadable or outside country table")
        );
        assert!(memory.verify().is_ok());
        assert_eq!(cache.allocated_bytes(), allocated);

        source.fault.set(Some(0x2808));
        let memory = cache.prepare(&source, 0, 0x1000);
        assert!(memory.authenticated_countries(0, 0x1000).is_none());
        assert!(matches!(
            memory.seed_status(),
            SeedStatus::Unavailable { .. }
        ));
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
        source.fault.set(None);

        // Metadata pressure disables only this optional capability, leaving
        // the byte cache, its status and full direct verification unchanged.
        cache.roster = vec![None; 1].into_boxed_slice();
        let memory = cache.prepare(&source, 0, 0x1000);
        assert_eq!(memory.seed_status(), SeedStatus::Ready);
        assert!(memory.authenticated_countries(0, 0x1000).is_none());
        assert_eq!(
            country_results(&memory, 0, 0x1000),
            country_results(&ByteOnly(&memory), 0, 0x1000)
        );
        assert!(memory.verify().is_ok());
    }

    #[test]
    fn authenticated_roster_never_elides_recording_wrapper_observations() {
        struct Recorder<'a, M> {
            source: &'a M,
            reads: RefCell<Vec<(usize, usize)>>,
        }
        impl<M: Memory> Memory for Recorder<'_, M> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, destination.len()));
                self.source.copy(address, destination)
            }
        }
        let source = Synthetic::countries();
        let mut cache = Cache::with_capacity(64);
        let memory = cache.prepare(&source, 0, 0x1000);
        let recorder = Recorder {
            source: &memory,
            reads: RefCell::new(Vec::new()),
        };
        assert!(recorder.authenticated_countries(0, 0x1000).is_none());
        let reader = ccountry::Reader::new(&recorder, 0, 0x1000).unwrap();
        assert!(reader.at(1).unwrap().is_some());
        assert!(reader.at(2).unwrap().is_none());
        assert_eq!(
            &*recorder.reads.borrow(),
            &[
                (0x1000 + cgame_state::COUNTRIES, 16),
                (0x1000 + cgame_state::COUNTRY_LINKS, 16),
                (0x1508, 8),
                (0x2000, 8),
                (0x2008, 4),
                (0x171c, 4),
                (0x1704, 4),
                (0x1704, 4),
                (0x1508, 8),
                (0x1510, 8),
                (0x2000, 8),
                (0x2008, 4),
                (0x171c, 4),
                (0x1708, 4),
                (0x1704, 4),
                (0x1508, 8),
            ],
        );
        assert!(memory.verify().is_ok());
    }

    #[test]
    fn authenticated_roster_keeps_typed_rows_and_capacity_outcomes() {
        for capacity in [0, 1, 2, 3] {
            let source = Synthetic::countries();
            for (offset, data) in [
                (ccountry::VOLUNTEER_HOSTS, 0x3000_u64),
                (ccountry::VOLUNTEER_TALLIES, 0x3100),
            ] {
                source.put(0x2000 + offset, &data.to_le_bytes());
                source.put(0x2000 + offset + 8, &2_u32.to_le_bytes());
                source.put(0x2000 + offset + 12, &2_u32.to_le_bytes());
            }
            source.put(0x3000, &7_u32.to_le_bytes());
            source.put(0x3004, &2_u32.to_le_bytes());
            source.put(0x3100, &31_i32.to_le_bytes());
            source.put(0x3104, &(-9_i32).to_le_bytes());
            let mut cache = Cache::with_capacity(64);
            let memory = cache.prepare(&source, 0, 0x1000);
            let mut actual = vec![ccountry::VolunteerHostRow::default(); capacity];
            let mut expected = actual.clone();
            source.observations.borrow_mut().clear();
            let actual_outcome = ccountry::walk_volunteer_hosts(&memory, 0, 0x1000, &mut actual);
            let actual_reads = source.observations.borrow().clone();
            source.observations.borrow_mut().clear();
            let expected_outcome =
                ccountry::walk_volunteer_hosts(&ByteOnly(&memory), 0, 0x1000, &mut expected);
            assert_eq!(actual_outcome, expected_outcome);
            assert_eq!(actual, expected);
            assert_eq!(&*source.observations.borrow(), &actual_reads);
            assert_eq!(
                actual_outcome,
                if capacity < 2 {
                    Outcome::Truncated {
                        len: capacity,
                        unread_rows: (2 - capacity) as u32,
                    }
                } else {
                    Outcome::Rows { len: 2 }
                }
            );
            assert!(memory.verify().is_ok());
        }
    }
}
