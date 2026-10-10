use super::*;
use crate::creference_registry::tests::Fixture;
use std::cell::{Cell, RefCell};

const COUNTRY: usize = 0x10000;
const POOL: usize = 0x30000;
const VARIANT: usize = 0x20000;
const HANDLE: u64 = (71_u64 << 32) | 70;
struct MemoryFixture {
    memory: Fixture,
    calls: RefCell<Vec<(usize, usize)>>,
}
impl Memory for MemoryFixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, output.len()));
        self.memory.copy(address, output)
    }
}
impl MemoryFixture {
    fn new() -> Self {
        let mut memory = Fixture::new(70);
        memory.put(0x1000 + creference_registry::COUNT, &1_u32.to_le_bytes());
        memory.entry(Fixture::index(HANDLE), 1, HANDLE, VARIANT as u64);
        memory.put(VARIANT, &(variant::VTABLE_RVA as u64).to_le_bytes());
        memory.put(VARIANT + 8, &HANDLE.to_le_bytes());
        memory.put(VARIANT + 0x198, &(-3_i64).to_le_bytes());
        memory.put(VARIANT + 0x1a0, &4_i64.to_le_bytes());
        memory.put(VARIANT + 0x1a8, &(-5_i64).to_le_bytes());
        memory.put(VARIANT + 0x208, &6_i64.to_le_bytes());
        let root = COUNTRY + ccountry::CONVOY_STATE;
        memory.put(root, &(super::super::VTABLE_RVA as u64).to_le_bytes());
        memory.put(root + super::super::COUNTRY, &7_u32.to_le_bytes());
        memory.put(
            root + super::super::EQUIPMENT_POOL,
            &(pool::VTABLE_RVA as u64).to_le_bytes(),
        );
        memory.put(root + super::super::RESERVATIONS, &[0; 16]);
        memory.put(COUNTRY + 0x1288, &(-8_i32).to_le_bytes());
        memory.put(0x0331_b7dc, &(-2_i32).to_le_bytes());
        Self {
            memory,
            calls: Default::default(),
        }
    }
    fn pool(&mut self, quantities: &[i64]) {
        let mut header = [0; 16];
        header[..8].copy_from_slice(&(POOL as u64).to_le_bytes());
        header[8..12].copy_from_slice(&(quantities.len() as u32).to_le_bytes());
        header[12..].copy_from_slice(&(quantities.len() as u32).to_le_bytes());
        self.memory.put(
            COUNTRY + ccountry::CONVOY_STATE + super::super::EQUIPMENT_POOL + pool::ITEMS,
            &header,
        );
        for (index, &quantity) in quantities.iter().enumerate() {
            self.memory
                .put(POOL + index * 16, &(VARIANT as u64).to_le_bytes());
            self.memory
                .put(POOL + index * 16 + 8, &quantity.to_le_bytes());
        }
    }
    fn attempts(&self, address: usize) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == address)
            .count()
    }
}
fn country() -> ccountry::Country {
    ccountry::Country {
        id: 7,
        canonical_slot: 1,
        pointer: COUNTRY,
    }
}

#[test]
fn dynamic_country_renumbering_keeps_the_original_embedded_convoy_inputs() {
    let mut memory = MemoryFixture::new();
    memory.pool(&[100_000]);
    let mut current = country();
    current.id = 5001;
    memory
        .memory
        .put(COUNTRY + ccountry::ID, &current.id.to_le_bytes());
    let mut scratch = Scratch::try_new(1).unwrap();
    let mut output = [PoolEntry::default(); 1];
    let original = Cache {
        valid: 1,
        raw: Some(100_000),
    };
    let value = capture(
        &memory,
        0,
        current,
        Some(Ok(original)),
        &mut output,
        &mut [],
        &mut scratch,
    );
    assert_eq!(value.cache, Ok(original));
    assert_eq!(value.pool, Ok(1));
    assert_eq!(value.reservations, Ok(0));
    assert!(value.first.unwrap().is_some());
    assert_eq!(
        memory.attempts(COUNTRY + ccountry::CONVOY_STATE + super::super::COUNTRY),
        0
    );
    assert_eq!(
        memory.attempts(COUNTRY + ccountry::CONVOY_STATE + super::super::CACHE_VALID),
        0
    );
}

#[test]
fn first_nonzero_negative_variant_is_unfiltered_and_keeps_signed_cached_stats() {
    let mut memory = MemoryFixture::new();
    memory.pool(&[0, -1, 100_000]);
    // There are deliberately no classifier bytes. Held-stock and first-variant selection never
    // require them, and an empty first occurrence cannot select the variant.
    let mut scratch = Scratch::try_new(1).unwrap();
    let mut output = [PoolEntry::default(); 3];
    let value = capture(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 1,
            raw: Some(9),
        })),
        &mut output,
        &mut [],
        &mut scratch,
    );
    assert_eq!(
        value.first,
        Ok(Some(Variant {
            variant: HANDLE,
            surface_detection: Ok(-3),
            submarine_detection: Ok(4),
            visibility: Ok(-5),
            speed: Ok(6)
        }))
    );
    assert_eq!(value.pool, Ok(3));
    assert_eq!(
        output
            .iter()
            .map(|entry| entry.quantity)
            .collect::<Vec<_>>(),
        [0, -1, 100_000]
    );
    assert!(output.iter().all(|entry| entry.flags.is_none()));
    assert_eq!(memory.attempts(VARIANT + variant::CLASSIFIER_FLAGS), 0);
    assert_eq!(memory.attempts(VARIANT + 0x198), 1);
    assert_eq!(
        memory.attempts(COUNTRY + ccountry::CONVOY_STATE + super::super::CACHE_VALID),
        0
    );
    scratch.retire();
    assert!(scratch.entries.is_empty() && scratch.registry.is_none());
}

#[test]
fn dirty_classifier_reads_zero_entries_and_failure_does_not_poison_first_selection() {
    let mut memory = MemoryFixture::new();
    memory.pool(&[0, -1, 100_000]);
    let mut scratch = Scratch::try_new(1).unwrap();
    let mut output = [PoolEntry::default(); 3];
    let value = capture(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 0,
            raw: None,
        })),
        &mut output,
        &mut [],
        &mut scratch,
    );
    assert!(value.classifier_failure.is_some());
    assert!(value.first.unwrap().is_some());
    assert_eq!(value.pool, Ok(3));
    assert_eq!(memory.attempts(VARIANT + variant::CLASSIFIER_FLAGS), 1);
    assert_eq!(
        memory.attempts(VARIANT),
        1,
        "identity/failure memos serve repeated occurrences"
    );
}

#[test]
fn original_cache_failure_and_successful_variant_stats_never_retry_inside_the_pass() {
    let mut memory = MemoryFixture::new();
    memory.pool(&[1]);
    let mut scratch = Scratch::try_new(1).unwrap();
    let mut output = [PoolEntry::default(); 1];
    let value = capture(
        &memory,
        0,
        country(),
        Some(Err("original held cache read failed")),
        &mut output,
        &mut [],
        &mut scratch,
    );
    assert_eq!(value.cache, Err("original held cache read failed"));
    assert_eq!(
        memory.attempts(COUNTRY + ccountry::CONVOY_STATE + super::super::CACHE_VALID),
        0
    );
    assert_eq!(
        memory.attempts(COUNTRY + ccountry::CONVOY_STATE + super::super::CACHE_RAW),
        0
    );
    memory.memory.put(VARIANT + 0x208, &99_i64.to_le_bytes());
    assert_eq!(scratch.stats(&memory, 0, VARIANT).unwrap().speed, Ok(6));
    assert_eq!(memory.attempts(VARIANT + 0x208), 1);
    scratch.begin();
    assert_eq!(scratch.stats(&memory, 0, VARIANT).unwrap().speed, Ok(99));
}

#[test]
fn first_failed_stat_copy_is_memoized_until_retirement() {
    struct OnceFault<'a> {
        memory: &'a MemoryFixture,
        fired: Cell<bool>,
    }
    impl Memory for OnceFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == VARIANT + 0x208 && !self.fired.replace(true) {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    let memory = MemoryFixture::new();
    let fault = OnceFault {
        memory: &memory,
        fired: Cell::new(false),
    };
    let mut scratch = Scratch::try_new(1).unwrap();
    let first = scratch.stats(&fault, 0, VARIANT).unwrap();
    assert_eq!(first.variant, HANDLE);
    assert_eq!(first.visibility, Ok(-5));
    assert_eq!(first.speed, Err("transport convoy speed cache unreadable"));
    assert_eq!(scratch.stats(&fault, 0, VARIANT).unwrap(), first);
    scratch.grow(5000).unwrap();
    assert_eq!(scratch.stats(&fault, 0, VARIANT).unwrap(), first);
    assert_eq!(memory.attempts(VARIANT + 0x208), 0);
    scratch.begin();
    assert_eq!(scratch.stats(&fault, 0, VARIANT).unwrap().speed, Ok(6));
}

#[test]
fn original_headers_grow_first_five_thousand_variant_pool_and_signed_reservations() {
    use crate::reference_bucket as bucket;
    const COUNT: usize = 5000;
    const REGISTRY: usize = 0x1000000;
    const MASK: u32 = 65535;
    let mut memory = MemoryFixture::new();
    memory.memory.put(
        0x1000 + creference_registry::ENTRIES,
        &(REGISTRY as u64).to_le_bytes(),
    );
    memory.memory.put(
        0x1000 + creference_registry::COUNT,
        &(COUNT as u32).to_le_bytes(),
    );
    memory
        .memory
        .put(0x1000 + creference_registry::MASK, &MASK.to_le_bytes());
    memory
        .memory
        .put(0x1000 + creference_registry::OVERFLOW_DISTANCE, &[18]);
    memory.memory.put(
        REGISTRY + (MASK as usize + 1 + 18) * bucket::STRIDE + bucket::DISTANCE,
        &[255],
    );
    memory.pool(&vec![100_000; COUNT]);
    let mut occupied = std::collections::BTreeSet::new();
    let mut handles = Vec::new();
    for id in 1_u64.. {
        let handle = (id << 32) | 70;
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(creference_registry::HASH_MULTIPLIER);
        let slot = ((hash >> 32) as u32 ^ hash as u32) & MASK;
        if !occupied.insert(slot) {
            continue;
        }
        let index = handles.len();
        let pointer = 0x400000 + index * 0x500;
        let at = REGISTRY + slot as usize * bucket::STRIDE;
        memory.memory.put(at, &[0; bucket::STRIDE]);
        memory.memory.put(at + bucket::DISTANCE, &[1]);
        memory.memory.put(at + bucket::TYPE, &handle.to_le_bytes());
        memory
            .memory
            .put(at + bucket::OBJECT, &(pointer as u64).to_le_bytes());
        memory
            .memory
            .put(pointer, &(variant::VTABLE_RVA as u64).to_le_bytes());
        memory.memory.put(pointer + 8, &handle.to_le_bytes());
        memory
            .memory
            .put(pointer + variant::CLASSIFIER_FLAGS, &1_u64.to_le_bytes());
        for (offset, value) in [(0x198, -3_i64), (0x1a0, 4), (0x1a8, -5), (0x208, 6)] {
            memory.memory.put(pointer + offset, &value.to_le_bytes());
        }
        memory
            .memory
            .put(POOL + index * 16, &(pointer as u64).to_le_bytes());
        handles.push(handle);
        if handles.len() == COUNT {
            break;
        }
    }
    let root = COUNTRY + ccountry::CONVOY_STATE;
    let mut header = [0; 16];
    header[..8].copy_from_slice(&0x280000_u64.to_le_bytes());
    header[8..12].copy_from_slice(&(COUNT as u32).to_le_bytes());
    header[12..].copy_from_slice(&(COUNT as u32).to_le_bytes());
    memory
        .memory
        .put(root + super::super::RESERVATIONS, &header);
    for index in 0..COUNT {
        let pointer = 0x2a0000 + index * 8;
        memory
            .memory
            .put(0x280000 + index * 8, &(pointer as u64).to_le_bytes());
        memory.memory.put(pointer, &(-(index as i32)).to_le_bytes());
        memory
            .memory
            .put(pointer + 4, &(i32::MIN + index as i32).to_le_bytes());
    }
    let mut pool = Vec::new();
    let mut reservations = Vec::new();
    let mut scratch = Scratch::vacant();
    let value = capture_growing(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 0,
            raw: None,
        })),
        (&mut pool, 0),
        (&mut reservations, 0),
        &mut scratch,
    );
    assert_eq!(value.pool, Ok(COUNT));
    assert_eq!(value.reservations, Ok(COUNT));
    assert_eq!(value.classifier_failure, None);
    assert_eq!(value.variant_identity_failure, None);
    assert_eq!(value.first.unwrap().unwrap().variant, handles[0]);
    assert_eq!(scratch.entries.len(), COUNT);
    assert_eq!(
        pool[..COUNT]
            .iter()
            .map(|entry| entry.variant.unwrap())
            .collect::<Vec<_>>(),
        handles
    );
    assert!(pool[..COUNT].iter().all(|entry| entry.flags == Some(1)));
    assert_eq!(
        reservations[COUNT - 1],
        Reservation {
            priority: -4999,
            quantity: i32::MIN + 4999
        }
    );
    assert_eq!(memory.attempts(root + super::super::RESERVATIONS), 1);
    assert_eq!(
        memory.attempts(root + super::super::EQUIPMENT_POOL + pool::ITEMS),
        1
    );
}

#[test]
fn each_failed_stat_leaf_keeps_authentic_first_identity_and_other_original_copies() {
    struct Fault<'a> {
        memory: &'a MemoryFixture,
        address: usize,
        calls: RefCell<Vec<usize>>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push(address);
            address != self.address && self.memory.copy(address, output)
        }
    }
    let mut memory = MemoryFixture::new();
    memory.pool(&[1, -1]);
    for (index, offset) in [0x1a8, 0x198, 0x1a0, 0x208].into_iter().enumerate() {
        let fault = Fault {
            memory: &memory,
            address: VARIANT + offset,
            calls: Default::default(),
        };
        let mut scratch = Scratch::try_new(1).unwrap();
        let mut output = [PoolEntry::default(); 2];
        let original = Some(Ok(Cache {
            valid: 1,
            raw: Some(100_000),
        }));
        let result = capture(
            &fault,
            0,
            country(),
            original,
            &mut output,
            &mut [],
            &mut scratch,
        );
        assert_eq!(result.pool, Ok(2));
        let first = result.first.unwrap().unwrap();
        assert_eq!(first.variant, HANDLE);
        let stats = [
            first.visibility,
            first.surface_detection,
            first.submarine_detection,
            first.speed,
        ];
        for (stat_index, stat) in stats.into_iter().enumerate() {
            assert_eq!(stat.is_err(), stat_index == index);
        }
        assert_eq!(scratch.stats(&fault, 0, VARIANT).unwrap(), first);
        for offset in [0x1a8, 0x198, 0x1a0, 0x208] {
            assert_eq!(
                fault
                    .calls
                    .borrow()
                    .iter()
                    .filter(|&&at| at == VARIANT + offset)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn later_nonzero_variant_identity_failure_does_not_erase_first_or_pool_quantities() {
    let mut memory = MemoryFixture::new();
    memory.pool(&[1, -1]);
    memory.memory.put(POOL + 16, &0_u64.to_le_bytes());
    let mut scratch = Scratch::try_new(2).unwrap();
    let mut output = [PoolEntry::default(); 2];
    let result = capture(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 1,
            raw: Some(7),
        })),
        &mut output,
        &mut [],
        &mut scratch,
    );
    assert_eq!(result.pool, Ok(2));
    assert_eq!(result.first.unwrap().unwrap().variant, HANDLE);
    assert_eq!(
        result.variant_identity_failure,
        Some("transport convoy variant is null")
    );
    assert_eq!(output[0].variant, Some(HANDLE));
    assert_eq!(
        output[1],
        PoolEntry {
            quantity: -1,
            ..Default::default()
        }
    );
}

#[test]
fn full_header_demand_survives_payload_capacity_refusal_and_empty_is_known() {
    let mut memory = MemoryFixture::new();
    memory.pool(&[1, -1, 0]);
    let mut scratch = Scratch::try_new(1).unwrap();
    let value = capture(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 0,
            raw: None,
        })),
        &mut [],
        &mut [],
        &mut scratch,
    );
    assert_eq!(value.pool_required, 3);
    assert!(value.pool.is_err() && value.first.is_err());
    assert_eq!(memory.attempts(POOL), 0);
    memory.pool(&[]);
    let value = capture(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 0,
            raw: None,
        })),
        &mut [],
        &mut [],
        &mut scratch,
    );
    assert_eq!(value.pool, Ok(0));
    assert_eq!(value.first, Ok(None));
    assert_eq!(value.reservations, Ok(0));
}

#[test]
fn signed_reservations_are_preserved_and_null_is_a_refusal() {
    let mut memory = MemoryFixture::new();
    memory.pool(&[]);
    let root = COUNTRY + ccountry::CONVOY_STATE;
    let mut header = [0; 16];
    header[..8].copy_from_slice(&0x40000_u64.to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..].copy_from_slice(&1_u32.to_le_bytes());
    memory
        .memory
        .put(root + super::super::RESERVATIONS, &header);
    memory.memory.put(0x40000, &0x50000_u64.to_le_bytes());
    memory.memory.put(0x50000, &(-7_i32).to_le_bytes());
    memory.memory.put(0x50004, &i32::MIN.to_le_bytes());
    let mut scratch = Scratch::try_new(1).unwrap();
    let mut reservations = [Reservation::default()];
    let value = capture(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 1,
            raw: Some(100_000),
        })),
        &mut [],
        &mut reservations,
        &mut scratch,
    );
    assert_eq!(value.reservations, Ok(1));
    assert_eq!(
        reservations[0],
        Reservation {
            priority: -7,
            quantity: i32::MIN
        }
    );
    memory.memory.put(0x40000, &0_u64.to_le_bytes());
    let value = capture(
        &memory,
        0,
        country(),
        Some(Ok(Cache {
            valid: 1,
            raw: Some(100_000),
        })),
        &mut [],
        &mut reservations,
        &mut scratch,
    );
    assert_eq!(
        value.reservations,
        Err("transport convoy reservation pointer is null")
    );
    assert_eq!(value.first, Ok(None));
}

#[test]
fn module_overflow_cannot_match_an_unreadable_embedded_identity() {
    let memory = MemoryFixture::new();
    let mut scratch = Scratch::try_new(1).unwrap();
    let value = capture(
        &memory,
        usize::MAX,
        country(),
        None,
        &mut [],
        &mut [],
        &mut scratch,
    );
    assert_eq!(
        value.cache,
        Err("transport embedded CConvoys module overflow")
    );
    assert_eq!(memory.attempts(COUNTRY + ccountry::CONVOY_STATE), 0);
}
