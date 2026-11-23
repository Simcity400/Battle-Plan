
use engine_allocation::SharedBox;
use std::cell::RefCell;

use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native::ccountry;

const MAX_BYTES: usize = 32;
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
    Capacity {
        required_cells: usize,
    },
    Unavailable {
        reason: &'static str,
    },
}

pub struct Cache {
    entries: Box<[Entry]>,
    filter: Box<[u64]>,
    front_filter: Box<[u64]>,
    filter_mask: usize,
    lengths: u64,
    len: usize,
    observed_reads: usize,
    status: SeedStatus,
    roster: Box<[Option<ccountry::Country>]>,
    roster_len: usize,
    roster_base: usize,
    roster_game_state: usize,
    roster_arrays: [Array; 2],
    reserved: Option<ccountry::ReservedCountryObservation>,
}

impl Cache {
    pub fn prepare_shared<M: Memory>(
        storage: &mut SharedBox<Self>,
        source: &M,
        base: usize,
        game_state: usize,
    ) -> Result<SharedBox<Self>, &'static str> {
        let cache =
            SharedBox::get_mut(storage).ok_or("country snapshot still leased by a reader")?;
        cache.prepare(source, base, game_state);
        Ok(SharedBox::clone(storage))
    }

    pub fn view<'a, M: Memory>(&'a self, source: &'a M) -> CountryMemory<'a, M> {
        CountryMemory {
            source,
            cache: self,
        }
    }

    pub fn roster_view<'a, M: Memory>(&'a self, source: &'a M) -> CountryMemory<'a, M, false> {
        CountryMemory {
            source,
            cache: self,
        }
    }

    pub fn bytes_view<'a, M: Memory>(&'a self, source: &'a M) -> CountryMemory<'a, M, true, false> {
        CountryMemory {
            source,
            cache: self,
        }
    }

    pub fn try_with_capacity(cells: usize) -> std::io::Result<Self> {
        let cells = if cells == 0 {
            0
        } else {
            cells
                .checked_next_power_of_two()
                .ok_or_else(engine_allocation::oom)?
        };
        let words = cells.div_ceil(u64::BITS as usize);
        Ok(Self {
            entries: engine_allocation::boxed_slice(engine_allocation::filled(
                cells,
                Entry::EMPTY,
            )?)?,
            filter: engine_allocation::boxed_slice(engine_allocation::filled(words, 0)?)?,
            front_filter: engine_allocation::boxed_slice(engine_allocation::filled(words, 0)?)?,
            filter_mask: (words * u64::BITS as usize).saturating_sub(1),
            lengths: 0,
            len: 0,
            observed_reads: 0,
            status: SeedStatus::Capacity { required_cells: 1 },
            roster: engine_allocation::boxed_slice(engine_allocation::filled(cells, None)?)?,
            roster_len: 0,
            roster_base: 0,
            roster_game_state: 0,
            roster_arrays: [Array { data: 0, count: 0 }; 2],
            reserved: None,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_capacity(cells: usize) -> Self {
        Self::try_with_capacity(cells).unwrap()
    }

    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    pub fn cells_for_countries(slots: usize, minimum: usize) -> usize {
        slots
            .saturating_mul(6)
            .saturating_add(2)
            .max(minimum)
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX)
    }

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
            + std::mem::size_of_val(&self.reserved)
    }

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
        self.reserved = None;
        if let Some(slot) = self.roster.first_mut() {
            *slot = None;
        }
        let result = {
            let recording = RecordingMemory {
                source,
                cache: RefCell::new(&mut *self),
            };
            (|| {
                let country_array = game_state
                    .checked_add(engine_native::cgame_state::COUNTRIES)
                    .and_then(|address| Array::read(&recording, address, 8))
                    .ok_or("country array unreadable or invalid");
                let root = match country_array {
                    Ok(array) => ccountry::cached_reserved_observation(source, base, game_state)
                        .unwrap_or_else(|| {
                            ccountry::observe_reserved_from_array(source, base, array)
                        }),
                    Err(reason) => ccountry::ReservedCountryObservation::from_result(Err(reason)),
                };
                {
                    let mut cache = recording.cache.borrow_mut();
                    cache.reserved = Some(root);
                    if let Some(slot) = cache.roster.first_mut() {
                        *slot = root.result().ok();
                    }
                }
                let country_array = country_array?;
                let links = game_state
                    .checked_add(engine_native::cgame_state::COUNTRY_LINKS)
                    .and_then(|address| Array::read(&recording, address, 4))
                    .ok_or("country links unreadable or invalid")?;
                let countries = ccountry::Reader::from_arrays(
                    &recording,
                    base,
                    game_state,
                    [country_array, links],
                )?;
                for slot in 1..countries.count() {
                    let country = countries.read_physical_at(slot)?;
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

    #[inline(never)]
    fn find_possible(&self, address: usize, len: usize) -> Option<&Entry> {
        let hash = Self::hash(address, len);
        let (word, bit) = self.filter_bit(hash);
        if self.filter[word] & bit == 0 {
            return None;
        }
        let first = hash & (self.entries.len() - 1);
        for probe in 0..self.entries.len() {
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
            for probe in 0..self.entries.len() {
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

struct RecordingMemory<'a, M> {
    source: &'a M,
    cache: RefCell<&'a mut Cache>,
}

impl<M: Memory> Memory for RecordingMemory<'_, M> {
    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        self.authenticated_reserved_country_observation(base, game_state)
            .map(ccountry::ReservedCountryObservation::result)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<ccountry::ReservedCountryObservation> {
        let cache = self.cache.borrow();
        (base == cache.roster_base && game_state == cache.roster_game_state)
            .then_some(cache.reserved)
            .flatten()
    }

    fn read_scalar<T: engine_native_memory::memory::Raw>(&self, address: usize) -> Option<T> {
        let value = self.source.read_scalar::<T>(address);
        let mut cache = self.cache.borrow_mut();
        if let Some(value) = value {
            cache.observe(address, engine_native_memory::memory::scalar_bytes(&value));
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

pub struct CountryMemory<'a, M, const CACHED_BYTES: bool = true, const ROSTER: bool = true> {
    source: &'a M,
    cache: &'a Cache,
}

impl<M: Memory, const CACHED_BYTES: bool, const ROSTER: bool>
    CountryMemory<'_, M, CACHED_BYTES, ROSTER>
{
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

impl<M: Memory, const CACHED_BYTES: bool, const ROSTER: bool> Memory
    for CountryMemory<'_, M, CACHED_BYTES, ROSTER>
{
    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        self.authenticated_reserved_country_observation(base, game_state)
            .map(ccountry::ReservedCountryObservation::result)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<ccountry::ReservedCountryObservation> {
        if !ROSTER {
            return self
                .source
                .authenticated_reserved_country_observation(base, game_state);
        }
        (base == self.cache.roster_base && game_state == self.cache.roster_game_state)
            .then_some(self.cache.reserved)
            .flatten()
    }

    #[inline]
    fn read_scalar<T: engine_native_memory::memory::Raw>(&self, address: usize) -> Option<T> {
        if CACHED_BYTES && let Some(entry) = self.cache.find(address, size_of::<T>()) {
            engine_native_memory::memory::scalar_from_bytes(&entry.bytes[..entry.len])
        } else {
            self.source.read_scalar(address)
        }
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        if !ROSTER {
            return self.source.authenticated_countries(base, game_state);
        }
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
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
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
                engine_native_memory::prefetch::span(address, destination.len());
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
