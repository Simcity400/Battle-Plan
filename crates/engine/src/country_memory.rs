//! Country authentication reused only within one measured hourly callback.
//!
//! A direct `CCountry` roster census seeds exact reads of headers, pointer slots,
//! links, vtables and identity words. The seed path cannot read country scalars.
//! Each preparation discards all previous observations. Every consumer remains
//! inside the quiescent hourly callback, before the trampoline resumes the game.
//! This cache is not a synchronization mechanism or a cross-hour address cache.

use std::{cell::RefCell, sync::Arc};

use crate::{Memory, ccountry, cpdx_array::Array};

const MAX_BYTES: usize = 32;
const MAX_PROBES: usize = 16;
/// Smallest filter; larger caches get four filter bits per cell so a mod-sized
/// census (tens of thousands of countries) keeps a useful rejection rate.
const FILTER_WORDS: usize = 256;
/// Distinct census reads per country slot, rounded up: the pointer, vtable,
/// identity and up to three link words, with headroom for the doubling rule.
const READS_PER_COUNTRY: usize = 8;

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

/// Reusable callback-owned storage, shared read-only with acquisition workers.
/// No lease may remain in a queued row buffer after the acquisition barrier.
pub struct Cache {
    entries: Box<[Entry]>,
    // A negative filter only: passing still requires the exact entry lookup.
    // It is rebuilt from successfully inserted keys during each preparation.
    filter: Box<[u64]>,
    // Cheap rejection before the mixed hash. Collisions only cause extra exact
    // lookups; this bitmap never supplies bytes or authenticates a country.
    front_filter: Box<[u64]>,
    /// Filter bit count minus one; both filters share the power-of-two size.
    filter_mask: usize,
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
    roster_arrays: [Array; 2],
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
    /// The owner retains this snapshot until all dependent readers finish.
    /// Every consumer must remain inside the quiescent hourly barrier.
    pub fn view<'a, M: Memory>(&'a self, source: &'a M) -> CountryMemory<'a, M> {
        CountryMemory {
            source,
            cache: self,
        }
    }

    /// Reuse the authenticated roster while ordinary reads retain the native
    /// path. The choice is compiled into each consumer, with no per-read probe.
    pub fn roster_view<'a, M: Memory>(&'a self, source: &'a M) -> CountryMemory<'a, M, false> {
        CountryMemory {
            source,
            cache: self,
        }
    }

    /// Allocate off the game thread. Zero disables storage; other capacities
    /// round up to a power of two for bounded, division-free lookup.
    pub fn try_with_capacity(cells: usize) -> std::io::Result<Self> {
        let cells = if cells == 0 {
            0
        } else {
            cells
                .checked_next_power_of_two()
                .ok_or_else(crate::allocation::oom)?
        };
        let words = (cells / 16).max(FILTER_WORDS);
        Ok(Self {
            entries: crate::allocation::boxed_slice(crate::allocation::filled(
                cells,
                Entry::EMPTY,
            )?)?,
            filter: crate::allocation::boxed_slice(crate::allocation::filled(words, 0)?)?,
            front_filter: crate::allocation::boxed_slice(crate::allocation::filled(words, 0)?)?,
            filter_mask: words * u64::BITS as usize - 1,
            lengths: 0,
            len: 0,
            observed_reads: 0,
            status: SeedStatus::Capacity { required_cells: 1 },
            roster: crate::allocation::boxed_slice(crate::allocation::filled(cells, None)?)?,
            roster_len: 0,
            roster_base: 0,
            roster_game_state: 0,
            roster_arrays: [Array { data: 0, count: 0 }; 2],
        })
    }

    #[cfg(test)]
    pub fn with_capacity(cells: usize) -> Self {
        Self::try_with_capacity(cells).unwrap()
    }

    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Cells for one complete census of `slots` country slots, so the first
    /// hour of a mod with tens of thousands of countries is already cached.
    /// The result never falls below `minimum`; growth stays with the caller.
    pub fn cells_for_countries(slots: usize, minimum: usize) -> usize {
        slots
            .saturating_mul(READS_PER_COUNTRY)
            .saturating_mul(2)
            .max(minimum)
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX)
    }

    /// Owned entry/filter storage plus derived roster storage and context.
    /// Excludes allocator metadata and the shared owner's Arc control block.
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of_val(self.entries.as_ref())
            + std::mem::size_of_val(self.filter.as_ref())
            + std::mem::size_of_val(self.front_filter.as_ref())
            + std::mem::size_of_val(&self.filter_mask)
            + std::mem::size_of_val(&self.lengths)
            + std::mem::size_of_val(self.roster.as_ref())
            + std::mem::size_of_val(&self.roster)
            + std::mem::size_of_val(&self.roster_len)
            + std::mem::size_of_val(&self.roster_base)
            + std::mem::size_of_val(&self.roster_game_state)
            + std::mem::size_of_val(&self.roster_arrays)
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
                Ok::<_, &'static str>((countries.count() as usize, countries.arrays()))
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
            && let Ok((count, arrays)) = result
            && count <= self.roster.len()
        {
            self.roster_len = count;
            self.roster_arrays = arrays;
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
    fn filter_bit(&self, hash: usize) -> (usize, u64) {
        let bit = hash & self.filter_mask;
        (
            bit / u64::BITS as usize,
            1_u64 << (bit % u64::BITS as usize),
        )
    }

    #[inline]
    fn front_filter_bit(&self, address: usize, len: usize) -> (usize, u64) {
        // No multiplication or division on the frequent non-country miss.
        // Fold alignment bits as well as higher address bits; distribution
        // affects performance only. Insertion and lookup use the same key.
        self.filter_bit(address ^ (address >> 3) ^ (address >> 17) ^ (len << 9))
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
        let (word, bit) = self.front_filter_bit(address, len);
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
        let (word, bit) = self.filter_bit(hash);
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
                    let (word, bit) = self.filter_bit(hash);
                    self.filter[word] |= bit;
                    let (word, bit) = self.front_filter_bit(address, bytes.len());
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
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        let value = self.source.read_scalar::<T>(address);
        let mut cache = self.cache.borrow_mut();
        if let Some(value) = value {
            cache.observe(address, crate::memory::scalar_bytes(&value));
        } else {
            cache.status = SeedStatus::Unavailable {
                reason: "country authentication source unreadable during census",
            };
        }
        value
    }

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

/// A single preparation's read-only view, borrowed within the hourly barrier.
pub struct CountryMemory<'a, M, const CACHED_BYTES: bool = true> {
    source: &'a M,
    cache: &'a Cache,
}

impl<M: Memory, const CACHED_BYTES: bool> CountryMemory<'_, M, CACHED_BYTES> {
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
}

impl<M: Memory, const CACHED_BYTES: bool> Memory for CountryMemory<'_, M, CACHED_BYTES> {
    #[inline]
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        if CACHED_BYTES && let Some(entry) = self.cache.find(address, size_of::<T>()) {
            crate::memory::scalar_from_bytes(&entry.bytes[..entry.len])
        } else {
            self.source.read_scalar(address)
        }
    }

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
                ccountry::AuthenticatedRoster::new(
                    &self.cache.roster[..self.cache.roster_len],
                    self.cache.roster_arrays,
                )
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
        // preserves their single guarded batch; country authentication stays
        // in the current callback's separately seeded snapshot.
        self.source.copy_records(sources, layout, destination)
    }

    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if CACHED_BYTES && let Some(entry) = self.cache.find(address, destination.len()) {
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
        if !CACHED_BYTES || self.cache.status != SeedStatus::Ready {
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

    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if CACHED_BYTES {
            for (&address, destination) in addresses.iter().zip(destinations.iter()) {
                if destination.is_empty() {
                    continue;
                }
                if address == 0 || address.checked_add(destination.len()).is_none() {
                    break;
                }
                crate::prefetch::span(address, destination.len());
            }
            self.copy_scattered(addresses, destinations)
        } else {
            self.source
                .copy_scattered_prefetched(addresses, destinations)
        }
    }
}

#[cfg(test)]
mod tests;
