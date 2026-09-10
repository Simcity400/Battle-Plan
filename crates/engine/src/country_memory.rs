//! Country authentication reused only within one measured hourly callback.
//!
//! A direct `CCountry` roster census seeds exact reads of headers, pointer slots,
//! links, vtables and identity words. The seed path cannot read country scalars.
//! Each preparation discards all previous observations; verification reads the
//! original memory again. This detects changed or newly unreadable cached bytes,
//! not a transient change that is restored before verification. The caller must
//! still use the proven stable boundary and invalidate affected tables when
//! verification fails. This cache is not a synchronization mechanism.

use std::cell::RefCell;

use crate::{Memory, ccountry};

const MAX_BYTES: usize = 32;
const MAX_PROBES: usize = 16;
const VERIFY_BATCH: usize = 64;

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

/// Reusable storage owned by one callback worker, never by a queued row buffer.
pub struct Cache {
    entries: Box<[Entry]>,
    len: usize,
    observed_reads: usize,
    status: SeedStatus,
}

impl Cache {
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
            len: 0,
            observed_reads: 0,
            status: SeedStatus::Capacity { required_cells: 1 },
        }
    }

    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of_val(self.entries.as_ref())
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
        self.len = 0;
        self.observed_reads = 0;
        self.status = SeedStatus::Ready;
        let result = {
            let recording = RecordingMemory {
                source,
                cache: RefCell::new(&mut *self),
            };
            (|| {
                let countries = ccountry::Reader::new(&recording, base, game_state)?;
                for slot in 1..countries.count() {
                    countries.at(slot)?;
                }
                Ok::<(), &'static str>(())
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
        CountryMemory {
            source,
            cache: self,
        }
    }

    fn start(&self, address: usize, len: usize) -> usize {
        // Mix the complete address: native object alignment must not turn the
        // many country vtables or identity words into one collision chain.
        let mut key = (address as u64) ^ (len as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        key ^= key >> 30;
        key = key.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        key ^= key >> 27;
        key = key.wrapping_mul(0x94d0_49bb_1331_11eb);
        (key ^ (key >> 31)) as usize & (self.entries.len() - 1)
    }

    fn find(&self, address: usize, len: usize) -> Option<&Entry> {
        if self.status != SeedStatus::Ready
            || len == 0
            || len > MAX_BYTES
            || self.entries.is_empty()
        {
            return None;
        }
        let first = self.start(address, len);
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
            let first = self.start(address, bytes.len());
            let mask = self.entries.len() - 1;
            for probe in 0..MAX_PROBES.min(self.entries.len()) {
                let entry = &mut self.entries[(first + probe) & mask];
                if entry.len == 0 {
                    entry.address = address;
                    entry.len = bytes.len();
                    entry.bytes[..bytes.len()].copy_from_slice(bytes);
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
    #[inline]
    fn copy_records(
        &self,
        pointers: &[usize],
        fields: &[crate::memory::RecordField],
        row_stride: usize,
        destination: &mut [u8],
    ) -> usize {
        // Fixed-layout gathers intentionally remain fresh source reads. This
        // preserves their single guarded batch; the separately seeded country
        // authentication observations are still verified after table walks.
        self.source
            .copy_records(pointers, fields, row_stride, destination)
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

    struct Synthetic {
        bytes: RefCell<Vec<u8>>,
        fault: Cell<Option<usize>>,
        guards: Cell<usize>,
        id_reads: Cell<usize>,
        change_id_during_seed: Cell<bool>,
    }

    impl Synthetic {
        fn countries() -> Self {
            let memory = Self {
                bytes: RefCell::new(vec![0; 0x3000]),
                fault: Cell::new(None),
                guards: Cell::new(0),
                id_reads: Cell::new(0),
                change_id_during_seed: Cell::new(false),
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
            assert_eq!(
                memory.copy_scattered(addresses, &mut [&mut first, &mut failed, &mut later]),
                1
            );
            assert_eq!(source.guards.get(), before + 1);
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
        assert_eq!(
            memory.copy_scattered([scalar, 0x2008], &mut [&mut failed, &mut later_hit]),
            0
        );
        assert_eq!(later_hit, [0xa5; 4]);
        source.fault.set(None);
        let mut empty = [];
        let mut id = [0; 4];
        let before = source.guards.get();
        assert_eq!(
            memory.copy_scattered([0, 0x2008], &mut [&mut empty, &mut id]),
            2
        );
        assert_eq!(source.guards.get(), before);
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
            assert_eq!(
                memory.verify(),
                Err("country authentication changed during table walks")
            );
            let source = Synthetic::countries();
            let memory = cache.prepare(&source, 0, 0x1000);
            source.fault.set(Some(address));
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
        assert_eq!(read::<u32>(&memory, 0x2008), Some(2));
        assert_eq!(memory.verify(), Ok(Verification::NotCached));
    }
}
