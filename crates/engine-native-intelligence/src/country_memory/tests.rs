use std::cell::Cell;

use super::*;
use engine_native::cgame_state;
use engine_native_memory::{read, walk::Outcome};

#[test]
fn reserved_observation_does_not_preauthenticate_unrelated_positive_objects() {
    let source = Synthetic::countries();
    source.put(0x1500, &0x2800_u64.to_le_bytes());
    source.put(0x2800, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    source.put(0x2800 + ccountry::ID, &0_u32.to_le_bytes());
    let reader = ccountry::Reader::new(&source, 0, 0x1000).unwrap();
    assert!(reader.at(1).unwrap().is_some());
    assert!(
        !source
            .observations
            .borrow()
            .iter()
            .any(|(address, _)| matches!(*address, 0x1500 | 0x2800 | 0x2808))
    );
    assert!(reader.reserved().is_ok());

    source.put(0x2000 + ccountry::ID, &0_u32.to_le_bytes());
    source.observations.borrow_mut().clear();
    assert_eq!(reader.at(1), Err("country id is zero"));
    assert_eq!(
        &*source.observations.borrow(),
        &[(0x1508, 8), (0x2000, 8), (0x2008, 4)]
    );
    source.put(0x1508, &0x2800_u64.to_le_bytes());
    source.observations.borrow_mut().clear();
    assert_eq!(reader.at(1), Ok(None));
    assert_eq!(&*source.observations.borrow(), &[(0x1508, 8)]);
}

#[test]
fn lazy_physical_id_zero_preserves_only_the_equal_selected_root_error() {
    struct FaultAt<'a> {
        source: &'a Synthetic,
        pointer: usize,
        fault_attempt: usize,
        attempts: Cell<usize>,
    }
    impl Memory for FaultAt<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == self.pointer {
                let current = self.attempts.get() + 1;
                self.attempts.set(current);
                if current == self.fault_attempt {
                    return false;
                }
            }
            self.source.copy(address, destination)
        }
    }
    for (selected, fault_attempt, expected) in [
        (0x2800, 2, "reserved country object identity mismatch"),
        (0x2c00, 1, "country id is zero"),
    ] {
        let source = Synthetic::countries();
        source.put(0x1500, &(selected as u64).to_le_bytes());
        source.put(0x1508, &0x2800_u64.to_le_bytes());
        for pointer in [0x2800, 0x2c00] {
            source.put(pointer, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
            source.put(pointer + ccountry::ID, &0_u32.to_le_bytes());
        }
        let memory = FaultAt {
            source: &source,
            pointer: selected,
            fault_attempt,
            attempts: Cell::new(0),
        };
        let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
        assert_eq!(memory.attempts.get(), 0);
        assert_eq!(reader.at(1), Err(expected));
        assert_eq!(
            reader.reserved(),
            Err("reserved country object identity mismatch")
        );
        assert_eq!(
            reader.reserved_observation().physical_alias(0x2800),
            (selected == 0x2800).then_some(Err("reserved country object identity mismatch"))
        );
        assert_eq!(memory.attempts.get(), fault_attempt);
        assert_eq!(reader.at(1), Err(expected));
        assert_eq!(memory.attempts.get(), fault_attempt);
    }
}

#[test]
fn reserved_failed_selection_is_scoped_and_reprepare_retires_it() {
    let source = Synthetic::countries();
    source.put(0x1500, &0x2800_u64.to_le_bytes());
    source.put(0x2800, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    source.put(0x2800 + ccountry::ID, &0_u32.to_le_bytes());
    source.fault.set(Some(0x2800));
    let mut cache = Cache::with_capacity(64);
    let view = cache.prepare(&source, 0, 0x1000);
    assert_eq!(view.seed_status(), SeedStatus::Ready);
    assert!(view.authenticated_countries(0, 0x1000).is_some());
    let root = view
        .authenticated_reserved_country_observation(0, 0x1000)
        .unwrap();
    assert_eq!(
        root.physical_alias(0x2800),
        Some(Err("reserved country object identity mismatch"))
    );
    assert!(
        view.authenticated_reserved_country_observation(1, 0x1000)
            .is_none()
    );
    assert!(
        view.authenticated_reserved_country_observation(0, 0x1100)
            .is_none()
    );

    source.fault.set(Some(0x1000 + cgame_state::COUNTRIES));
    let view = cache.prepare(&source, 0, 0x1000);
    assert!(matches!(view.seed_status(), SeedStatus::Unavailable { .. }));
    let root = view
        .authenticated_reserved_country_observation(0, 0x1000)
        .unwrap();
    assert_eq!(root.result(), Err("country array unreadable or invalid"));
    assert_eq!(root.physical_alias(0x2800), None);

    source.fault.set(None);
    source.put(0x1500, &0x2c00_u64.to_le_bytes());
    source.put(0x2c00, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    source.put(0x2c00 + ccountry::ID, &0_u32.to_le_bytes());
    let view = cache.prepare(&source, 0, 0x1000);
    assert_eq!(view.seed_status(), SeedStatus::Ready);
    let root = view
        .authenticated_reserved_country_observation(0, 0x1000)
        .unwrap();
    assert_eq!(root.physical_alias(0x2800), None);
    assert_eq!(root.physical_alias(0x2c00), Some(Ok(())));
}

#[test]
fn shared_snapshot_refuses_reseed_while_leased_without_reading_or_mutating() {
    let source = Synthetic::countries();
    let mut owner = engine_allocation::shared(Cache::with_capacity(64)).unwrap();
    let first = Cache::prepare_shared(&mut owner, &source, 0, 0x1000).unwrap();
    let initial = shared_walks(&first.view(&source));
    source.observations.borrow_mut().clear();
    let worker = first.clone();
    assert_eq!(
        Cache::prepare_shared(&mut owner, &source, 0, 0x1000).err(),
        Some("country snapshot still leased by a reader")
    );
    assert!(source.observations.borrow().is_empty());
    assert_eq!(shared_walks(&worker.view(&source)), initial);
    drop(worker);
    drop(first);
    let next = Cache::prepare_shared(&mut owner, &source, 0, 0x1000).unwrap();
    assert_eq!(shared_walks(&next.view(&source)), initial);
}

#[test]
fn roster_view_keeps_scoped_authentication_and_fresh_faulted_byte_reads() {
    let source = Synthetic::countries();
    let expected = shared_walks(&source);
    let mut cache = Cache::with_capacity(64);
    cache.prepare(&source, 0, 0x1000);
    let memory = cache.roster_view(&source);
    assert_eq!(shared_walks(&memory), expected);
    assert!(memory.authenticated_countries(0, 0x1000).is_some());
    assert!(memory.authenticated_countries(1, 0x1000).is_none());
    assert!(memory.authenticated_countries(0, 0x1100).is_none());

    let identity = 0x2000 + ccountry::ID;
    source.put(identity, &99_u32.to_le_bytes());
    let scalar_before = source.scalar_reads.get();
    assert_eq!(read::<u32>(&memory, identity), Some(99));
    assert_eq!(source.scalar_reads.get(), scalar_before + 1);
    assert_eq!(read::<u32>(&cache.view(&source), identity), Some(7));
    assert_eq!(source.scalar_reads.get(), scalar_before + 1);
    source.fault.set(Some(identity));
    assert_eq!(read::<u32>(&memory, identity), None);
    assert_eq!(source.scalar_reads.get(), scalar_before + 2);
    for prefetched in [false, true] {
        let mut first = [0; 8];
        let mut failed = [0x55; 4];
        let mut later = [0x66; 4];
        let addresses = [0x2000, identity, 0x2000 + ccountry::COMMAND_POWER];
        let mut destinations: [&mut [u8]; 3] = [&mut first, &mut failed, &mut later];
        let completed = if prefetched {
            memory.copy_scattered_prefetched(addresses, &mut destinations)
        } else {
            memory.copy_scattered(addresses, &mut destinations)
        };
        assert_eq!(completed, 1);
        assert_eq!(first, (ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
        assert_eq!(failed, [0x7e, 0x55, 0x55, 0x55]);
        assert_eq!(later, [0x66; 4]);
    }
    source.fault.set(None);
    source.fault.set(Some(0x1000 + cgame_state::COUNTRIES));
    cache.prepare(&source, 0, 0x1000);
    assert!(
        cache
            .roster_view(&source)
            .authenticated_countries(0, 0x1000)
            .is_none()
    );
}

#[test]
fn roster_headers_and_proved_links_remove_guards_without_reading_ordinary_fields() {
    fn queries(memory: &impl Memory) -> (Vec<Option<engine_native_memory::country_identity::Country>>, Vec<engine_native_memory::country_identity::Country>) {
        let reader = ccountry::Reader::new(memory, 0, 0x1000).unwrap();
        (
            (1..reader.count())
                .map(|slot| reader.at(slot).unwrap())
                .collect(),
            [1, 2, 7].map(|raw| reader.resolve(raw).unwrap()).to_vec(),
        )
    }

    let source = Synthetic::countries();
    let expected = queries(&source);
    let direct_guards = source.guards.replace(0);
    let direct_bytes: usize = source
        .observations
        .borrow()
        .iter()
        .map(|(_, len)| len)
        .sum();
    assert_eq!(direct_guards, 36);
    assert_eq!(direct_bytes, 252);
    let mut cache = Cache::with_capacity(64);
    cache.prepare(&source, 0, 0x1000);
    source.guards.set(0);
    source.observations.borrow_mut().clear();
    let memory = cache.roster_view(&source);
    assert_eq!(queries(&memory), expected);
    assert_eq!(source.guards.get(), 1);
    assert_eq!(&*source.observations.borrow(), &[(0x171c, 4)]);
    let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
    for raw in [3_u32, 4] {
        let address = 0x1700 + raw as usize * 4;
        for value in [0_u32, 1, 4] {
            source.put(address, &value.to_le_bytes());
            source.observations.borrow_mut().clear();
            assert_eq!(
                reader.link(raw),
                match value {
                    0 => Ok(None),
                    1 => Ok(Some(1)),
                    _ => Err("country link unreadable or outside country table"),
                }
            );
            assert_eq!(&*source.observations.borrow(), &[(address, 4)]);
        }
        source.fault.set(Some(address));
        assert_eq!(
            reader.link(raw),
            Err("country link unreadable or outside country table")
        );
        source.fault.set(None);
    }
    source.observations.borrow_mut().clear();
    assert_eq!(reader.link(0), Err("country id is zero"));
    assert_eq!(
        reader.link(8),
        Err("country link unreadable or outside country table")
    );
    assert!(source.observations.borrow().is_empty());

    source.put(0x1000 + cgame_state::COUNTRIES, &0x1580_u64.to_le_bytes());
    source.put(
        0x1000 + cgame_state::COUNTRY_LINKS,
        &0x1780_u64.to_le_bytes(),
    );
    for slot in [1, 2] {
        source.put(0x1580 + slot * 8, &0x2000_u64.to_le_bytes());
    }
    for raw in [1, 2, 7] {
        source.put(0x1780 + raw * 4, &1_u32.to_le_bytes());
    }
    cache.prepare(&source, 0, 0x1000);
    source.observations.borrow_mut().clear();
    assert_eq!(queries(&cache.roster_view(&source)), expected);
    assert_eq!(&*source.observations.borrow(), &[(0x179c, 4)]);
}

#[test]
fn absent_roster_headers_preserve_the_original_native_failure_path() {
    for capacity in [0, 1, 64] {
        for fault in [
            0x1000 + cgame_state::COUNTRIES,
            0x1000 + cgame_state::COUNTRY_LINKS + 15,
            0x1508,
            0x2000,
            0x2008,
            0x1704,
        ] {
            let source = Synthetic::countries();
            source.fault.set(Some(fault));
            let mut cache = Cache::with_capacity(capacity);
            cache.prepare(&source, 0, 0x1000);
            let memory = cache.roster_view(&source);
            assert!(memory.authenticated_countries(0, 0x1000).is_none());
            source.observations.borrow_mut().clear();
            let expected = country_results(&source, 0, 0x1000);
            let expected_reads = source.observations.take();
            assert_eq!(country_results(&memory, 0, 0x1000), expected);
            assert_eq!(*source.observations.borrow(), expected_reads);
        }
    }
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
            let (word, bit) = cache.front_filter_bit(address, len);
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
    let bytes = (ccountry::VTABLE_RVA.0 as u64).to_le_bytes();
    cache.observe(0x2000, &bytes);
    let key = cache.front_filter_bit(0x2000, 8);
    let collision = (1..=(cache.filter_mask + 1) * 8)
        .find(|&address| address != 0x2000 && cache.front_filter_bit(address, 8) == key)
        .expect("finite bitmap permits collisions");
    assert!(cache.find(collision, 8).is_none());
    source.fault.set(Some(collision));
    let memory = CountryMemory::<_> {
        source: &source,
        cache: &cache,
    };
    let mut output = [0; 8];
    assert!(!memory.copy(collision, &mut output));
    assert_eq!(source.observations.borrow().last(), Some(&(collision, 8)));

    let mut changed = bytes;
    changed[0] ^= 1;
    cache.observe(0x2000, &changed);
    assert!(matches!(cache.status, SeedStatus::Unavailable { .. }));
    assert!(cache.find(0x2000, 8).is_none());
}

struct Synthetic {
    bytes: RefCell<Vec<u8>>,
    fault: Cell<Option<usize>>,
    guards: Cell<usize>,
    scalar_reads: Cell<usize>,
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
            scalar_reads: Cell::new(0),
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
        for slot in [1, 2] {
            memory.put(0x1500 + slot * 8, &0x2000_u64.to_le_bytes());
        }
        for link in [1, 2, 7] {
            memory.put(0x1700 + link * 4, &1_u32.to_le_bytes());
        }
        memory.put(0x2000, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
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
        engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &self.bytes.borrow(),
        }
        .copy(address, destination)
    }
}

impl Memory for Synthetic {
    fn read_scalar<T: engine_native_memory::memory::Raw>(&self, address: usize) -> Option<T> {
        self.scalar_reads.set(self.scalar_reads.get() + 1);
        let mut bytes = vec![0; size_of::<T>()];
        if !self.copy(address, &mut bytes) {
            return None;
        }
        engine_native_memory::memory::scalar_from_bytes(&bytes)
    }

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

fn shared_walks(memory: &impl Memory) -> (Vec<engine_native_memory::country_identity::Country>, Vec<ccountry::LinkRow>) {
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
    let mut cache = Cache::with_capacity(64);
    let filter_bits = cache.filter_mask + 1;
    let mut seen = vec![None; filter_bits];
    let (first, second) = (0..=filter_bits)
        .find_map(|index| {
            let address = 0x1000 + index * 0x1000;
            let bit = Cache::hash(address, 8) & cache.filter_mask;
            seen[bit]
                .replace(address)
                .map(|previous| (previous, address))
        })
        .expect("more distinct keys than filter bits must collide");
    cache.status = SeedStatus::Ready;
    cache.observe(first, &[0x41; 8]);
    assert_eq!(
        cache.filter_bit(Cache::hash(first, 8)),
        cache.filter_bit(Cache::hash(second, 8))
    );
    assert!(cache.find(second, 8).is_none());
    cache.observe(second, &[0x72; 8]);
    cache.observe(first, &[0x41; 8]);
    assert_eq!(cache.len, 2);
    assert_eq!(cache.lengths, 1 << 8);
    assert_eq!(&cache.find(first, 8).unwrap().bytes[..8], &[0x41; 8]);
    assert_eq!(&cache.find(second, 8).unwrap().bytes[..8], &[0x72; 8]);

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
        256 * (std::mem::size_of::<Entry>() + std::mem::size_of::<Option<engine_native_memory::country_identity::Country>>())
            + 2 * 4 * std::mem::size_of::<u64>()
            + std::mem::size_of::<usize>()
            + std::mem::size_of::<u64>()
            + std::mem::size_of::<Box<[Option<engine_native_memory::country_identity::Country>]>>()
            + 3 * std::mem::size_of::<usize>()
            + std::mem::size_of::<[Array; 2]>()
            + std::mem::size_of::<Option<engine_native_memory::country_identity::ReservedCountryObservation>>()
    );
    assert_eq!(
        Cache::with_capacity(0).allocated_bytes(),
        std::mem::size_of::<usize>()
            + std::mem::size_of::<u64>()
            + std::mem::size_of::<Box<[Option<engine_native_memory::country_identity::Country>]>>()
            + 3 * std::mem::size_of::<usize>()
            + std::mem::size_of::<[Array; 2]>()
            + std::mem::size_of::<Option<engine_native_memory::country_identity::ReservedCountryObservation>>()
    );
}

#[test]
fn preparations_replace_all_negative_filter_bits_and_old_keys() {
    let source = Synthetic::countries();
    let mut cache = Cache::with_capacity(64);
    let memory = cache.prepare(&source, 0, 0x1000);
    assert_eq!(memory.seed_status(), SeedStatus::Ready);
    cache.filter.fill(u64::MAX);
    cache.lengths = u64::MAX;
    for slot in [1, 2] {
        source.put(0x1500 + slot * 8, &0x2800_u64.to_le_bytes());
    }
    source.put(0x2800, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    source.put(0x2808, &7_u32.to_le_bytes());
    source.put(0x2008, &99_u32.to_le_bytes());
    let memory = cache.prepare(&source, 0, 0x1000);
    assert_eq!(memory.seed_status(), SeedStatus::Ready);
    assert_eq!(memory.cache.lengths, (1 << 4) | (1 << 8) | (1 << 16));
    let mut expected = vec![0; memory.cache.filter.len()];
    for entry in memory.cache.entries.iter().filter(|entry| entry.len != 0) {
        let (word, bit) = memory
            .cache
            .filter_bit(Cache::hash(entry.address, entry.len));
        expected[word] |= bit;
        assert!(memory.cache.find(entry.address, entry.len).is_some());
    }
    assert_eq!(&*memory.cache.filter, &expected);
    assert!(memory.cache.find(0x2008, 4).is_none());
    let before = source.guards.get();
    let scalar_before = source.scalar_reads.get();
    assert!(scalar_before > 0);
    assert_eq!(read::<u32>(&memory, 0x2008), Some(99));
    assert_eq!(source.guards.get(), before + 1);
    assert_eq!(source.scalar_reads.get(), scalar_before + 1);
    assert_eq!(read::<u32>(&memory, 0x2808), Some(7));
    assert_eq!(source.guards.get(), before + 1);
    assert_eq!(source.scalar_reads.get(), scalar_before + 1);
    assert_eq!(read::<[u8; 0]>(&memory, 0), Some([]));
    assert_eq!(read::<u32>(&memory, 0), None);
    assert_eq!(read::<u32>(&memory, usize::MAX - 3), None);
    assert_eq!(source.scalar_reads.get(), scalar_before + 1);

    source.fault.set(Some(0x1000 + cgame_state::COUNTRIES));
    let memory = cache.prepare(&source, 0, 0x1000);
    assert!(matches!(
        memory.seed_status(),
        SeedStatus::Unavailable { .. }
    ));
    assert!(memory.cache.filter.iter().all(|&word| word == 0));
    assert_eq!(memory.cache.lengths, 0);
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
    assert_eq!(source.guards.get() - seed_cost, 4 * 32);
    assert!(source.guards.get() < direct_cost * 32);
    let scalar = i64::MIN + 987;
    source.put(0x2000 + ccountry::COMMAND_POWER, &scalar.to_le_bytes());
    assert_eq!(
        read::<i64>(&memory, 0x2000 + ccountry::COMMAND_POWER),
        Some(scalar)
    );
    source.put(0x2000 + ccountry::SURRENDER_TARGET, &u32::MAX.to_le_bytes());
    assert_eq!(read::<u64>(&memory, 0x2008), Some(0xffff_ffff_0000_0007));
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
        let scalar_before = source.scalar_reads.get();
        assert_eq!(read::<u32>(&memory, 0x2008), None);
        assert_eq!(source.scalar_reads.get(), scalar_before + 1);
        let mut failed = [0xa5; 4];
        assert!(!memory.copy(0x2008, &mut failed));
        assert_eq!(failed, [0x7e, 0xa5, 0xa5, 0xa5]);
        source.fault.set(None);
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
    let memory = cache.prepare(&source, 0, 0x1000);
    assert_eq!(memory.seed_status(), SeedStatus::Ready);
    assert_eq!(read::<u32>(&memory, 0x2008), Some(2));
    let memory = cache.prepare(&source, 0, usize::MAX);
    assert!(matches!(
        memory.seed_status(),
        SeedStatus::Unavailable { .. }
    ));
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
    let scalar_before = source.scalar_reads.get();
    assert!(scalar_before > 0);
    assert_eq!(read::<u32>(&memory, 0x2008), Some(2));
    assert_eq!(source.scalar_reads.get(), scalar_before + 1);
}

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
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.0.copy_records(sources, layout, destination)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CountryResults {
    slots: Vec<Result<Option<engine_native_memory::country_identity::Country>, &'static str>>,
    resolved: Vec<Result<engine_native_memory::country_identity::Country, &'static str>>,
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
fn authenticated_roster_matches_byte_cache_first_reads_and_faults() {
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
    let country = engine_native_memory::country_identity::Country {
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
    assert_eq!(memory.cached_entries(), 10);
}

#[test]
fn authenticated_roster_reprepare_replaces_aliases_nulls_and_short_storage() {
    let source = Synthetic::countries();
    let mut cache = Cache::with_capacity(64);
    let allocated = cache.allocated_bytes();
    let memory = cache.prepare(&source, 0, 0x1000);
    assert!(memory.authenticated_countries(0, 0x1000).is_some());

    source.put(0x1508, &0x2800_u64.to_le_bytes());
    source.put(0x1510, &0_u64.to_le_bytes());
    source.put(0x2800, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    source.put(0x2808, &1_u32.to_le_bytes());
    let memory = cache.prepare(&source, 0, 0x1000);
    let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(
        reader.at(1),
        Ok(Some(engine_native_memory::country_identity::Country {
            pointer: 0x2800,
            canonical_slot: 1,
            id: 1
        }))
    );
    assert_eq!(reader.at(2), Ok(None));
    assert_eq!(reader.resolve(2), reader.resolve(1));

    source.put(0x1000 + cgame_state::COUNTRIES + 12, &2_u32.to_le_bytes());
    let memory = cache.prepare(&source, 0, 0x1000);
    assert!(memory.authenticated_countries(0, 0x1000).is_some());
    let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(reader.count(), 2);
    assert_eq!(
        reader.at(2),
        Err("country pointer unreadable or outside country table")
    );
    assert_eq!(cache.allocated_bytes(), allocated);

    source.fault.set(Some(0x2808));
    let memory = cache.prepare(&source, 0, 0x1000);
    assert!(memory.authenticated_countries(0, 0x1000).is_none());
    assert!(matches!(
        memory.seed_status(),
        SeedStatus::Unavailable { .. }
    ));
    source.fault.set(None);

    cache.roster = vec![None; 1].into_boxed_slice();
    let memory = cache.prepare(&source, 0, 0x1000);
    assert_eq!(memory.seed_status(), SeedStatus::Ready);
    assert!(memory.authenticated_countries(0, 0x1000).is_none());
    assert_eq!(
        country_results(&memory, 0, 0x1000),
        country_results(&ByteOnly(&memory), 0, 0x1000)
    );
}

#[test]
fn authenticated_roster_keeps_typed_rows_from_any_starting_storage() {
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
        let actual_outcome = ccountry::walk_volunteer_hosts_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut actual));
        let actual_reads = source.observations.borrow().clone();
        source.observations.borrow_mut().clear();
        let expected_outcome =
            ccountry::walk_volunteer_hosts_buffer(&ByteOnly(&memory), 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut expected));
        assert_eq!(actual_outcome, expected_outcome);
        assert_eq!(actual, expected);
        assert_eq!(&*source.observations.borrow(), &actual_reads);
        assert_eq!(actual_outcome, Outcome::Rows { len: 2 }, "capacity {capacity}");
    }
}
