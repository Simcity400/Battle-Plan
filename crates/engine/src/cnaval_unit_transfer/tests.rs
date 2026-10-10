use super::*;
use crate::{cstrategic_navy, memory::SliceMemory};
use std::collections::BTreeMap;

#[test]
fn allocation_capacity_overflow_preserves_movement_memo() {
    let mut memo = MovementSpeeds::new(8);
    let before = memo.allocated_bytes();
    assert_eq!(
        memo.reserve(usize::MAX).unwrap_err().kind(),
        std::io::ErrorKind::OutOfMemory,
    );
    assert_eq!(memo.allocated_bytes(), before);
}

#[test]
fn vacant_legacy_movement_memo_keeps_healthy_uncached_speed() {
    let source = fixture();
    let mut memo = MovementSpeeds::vacant();
    assert_eq!(
        memo.get(&source, 0, 0x1000, 1, 0xd000),
        movement_speed(&source, 0, 0x1000, 1, 0xd000)
    );
}

#[test]
fn transport_success_cache_saturation_keeps_scoped_refusal_and_shared_root() {
    use std::cell::{Cell, RefCell};
    struct Fault<'a> {
        source: &'a Fixture,
        failed: Cell<bool>,
        reads: RefCell<Vec<usize>>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            if address == 0x14008 && !self.failed.replace(true) {
                return false;
            }
            self.source.copy(address, output)
        }
    }
    let mut source = fixture();
    source.word(0xc200, 0xd100);
    source.word(0xd100, cstrategic_region::VTABLE_RVA as u64);
    source.dword(0xd100 + cstrategic_region::ID, 0);
    source.dword(0xd100 + cstrategic_region::RUNTIME_SLOT, 0);
    source.word(0xf000, sweather_per_region::VTABLE_RVA as u64);
    source.dword(0xf000 + sweather_per_region::ID, 0);
    source.word(0x14000, 100_000);
    let memory = Fault {
        source: &source,
        failed: Cell::new(false),
        reads: RefCell::new(Vec::new()),
    };
    let mut memo = MovementSpeeds::new(1);
    memo.reserve_queries(3, 2).unwrap();
    // Existing optional success-cache saturation uses the uncached path.
    for index in 0..memo.observations.capacity() {
        let input = Input::Country(10_000 + index as u32);
        memo.observations.push(Observation {
            input,
            result: Ok(InputValue::Scalar(0)),
        });
        memo.index_observation(input, index * 2 + 1);
    }
    assert_eq!(
        memo.get_for_region(&memory, 0, 0x1000, 1, 0xd000, 1),
        Err("navy regional operand unreadable")
    );
    let expected = movement_speed(&source, 0, 0x1000, 1, 0xd100);
    assert_eq!(
        memo.get_for_region(&memory, 0, 0x1000, 1, 0xd100, 0),
        expected
    );
    assert_eq!(
        memo.get_for_region(&memory, 0, 0x1000, 1, 0xd000, 1),
        Err("navy regional operand unreadable")
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == 0x14008)
            .count(),
        1
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == BASE_SPEED_RVA)
            .count(),
        1
    );
}

struct Fixture {
    bytes: Vec<u8>,
    globals: BTreeMap<usize, Vec<u8>>,
}
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some((&start, bytes)) = self.globals.range(..=address).next_back()
            && let Some(bytes) = bytes.get(address - start..address - start + output.len())
        {
            output.copy_from_slice(bytes);
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}
impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn dword(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, pointer: u64, count: u32) {
        self.word(at, pointer);
        self.dword(at + 8, count);
        self.dword(at + 12, count);
    }
    fn global(&mut self, at: usize, value: i64) {
        self.globals.insert(at, value.to_le_bytes().to_vec());
    }
    fn modifier(&mut self, at: usize, data: usize, token: u32, value: i64) {
        self.array(at, data as u64, 1);
        self.dword(data, token);
        self.word(data + 8, value as u64);
    }
}
fn fixture() -> Fixture {
    let mut m = Fixture {
        bytes: vec![0; 0x40000],
        globals: BTreeMap::new(),
    };
    m.array(0x1000 + cgame_state::COUNTRIES, 0x2000, 3);
    m.array(0x1000 + cgame_state::COUNTRY_LINKS, 0x2100, 3);
    for (id, country, diplomacy, intel) in
        [(1, 0x3000, 0xa000, 0x8000), (2, 0x5000, 0xb000, 0x9000)]
    {
        m.word(0x2000 + id * 8, country as u64);
        m.dword(0x2100 + id * 4, id as u32);
        m.word(country, ccountry::VTABLE_RVA as u64);
        m.dword(country + ccountry::ID, id as u32);
        m.word(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        m.word(diplomacy, cdiplomacy_status::VTABLE_RVA as u64);
        m.word(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
        m.word(country + ccountry::INTEL_TABLE, intel as u64);
        m.word(intel, ccountry_intel::VTABLE_RVA as u64);
        m.dword(intel + ccountry_intel::OWNER, id as u32);
        m.array(intel + ccountry_intel::DATA, (intel + 0x100) as u64, 3);
    }
    m.array(0xa000 + cdiplomacy_status::ENEMY_COUNTRIES, 0xc000, 1);
    m.dword(0xc000, 2);
    m.dword(0x5000 + ccountry::FOG_NETWORK_TABLE_COUNT, 1);
    m.word(
        0x8100 + 2 * country_intel_row::STRIDE + country_intel_row::NAVY,
        30_000,
    );
    m.word(
        0x9100 + country_intel_row::STRIDE + country_intel_row::NAVY,
        10_000,
    );
    m.modifier(
        0x3000 + ccountry::MODIFIER_ACCUMULATOR,
        0xc100,
        REDUCTION_TOKEN,
        20_000,
    );
    m.array(0x1000 + cgame_state::STRATEGIC_REGIONS, 0xc200, 2);
    m.word(0xc208, 0xd000);
    m.word(0xd000, cstrategic_region::VTABLE_RVA as u64);
    m.dword(0xd000 + cstrategic_region::ID, 1);
    m.dword(0xd000 + cstrategic_region::RUNTIME_SLOT, 1);
    m.word(0x1000 + cgame_state::WEATHER_MANAGER, 0xe000);
    m.word(0xe000, cweather_manager::VTABLE_RVA as u64);
    m.array(0xe000 + cweather_manager::REGIONS, 0xf000, 2);
    let weather = 0xf000 + sweather_per_region::STRIDE;
    m.word(weather, sweather_per_region::VTABLE_RVA as u64);
    m.dword(weather + sweather_per_region::ID, 1);
    m.modifier(
        weather + sweather_per_region::MODIFIERS,
        0x10000,
        SPEED_TOKEN,
        -10_000,
    );
    m.word(0x1000 + cgame_state::STRATEGIC_NAVY_MANAGER, 0x11000);
    m.word(0x11000, cstrategic_navy_manager::VTABLE_RVA as u64);
    m.array(0x11000 + cstrategic_navy_manager::NAVIES, 0x12000, 3);
    m.word(0x12008, 0x18000);
    m.word(0x18000, cstrategic_navy::VTABLE_RVA as u64);
    m.word(
        0x18000 + cstrategic_navy::SECONDARY,
        cstrategic_navy::SECONDARY_VTABLE_RVA as u64,
    );
    m.dword(0x18000 + cstrategic_navy::OWNER, 1);
    // Deliberately invalid: native does not consume the OWNER's operand.
    m.array(0x18000 + cstrategic_navy::REGIONAL_OPERANDS, 0x50000, 2);
    m.word(0x12010, 0x13000);
    m.word(0x13000, cstrategic_navy::VTABLE_RVA as u64);
    m.word(
        0x13000 + cstrategic_navy::SECONDARY,
        cstrategic_navy::SECONDARY_VTABLE_RVA as u64,
    );
    m.dword(0x13000 + cstrategic_navy::OWNER, 2);
    m.array(0x13000 + cstrategic_navy::REGIONAL_OPERANDS, 0x14000, 2);
    m.word(0x14008, 50_000);
    m.global(MODIFIER_SET_RVA, 0x15000);
    m.word(0x15000 + TRANSPORT_MODIFIER, 0x16000);
    m.word(0x16000, cstatic_modifier::VTABLE_RVA as u64);
    m.modifier(
        0x16000 + cstatic_modifier::ACCUMULATOR,
        0x17000,
        SPEED_TOKEN,
        -50_000,
    );
    m.global(modifier_definition::TABLE_RVA, 0x20000);
    m.dword(
        0x20000
            + SPEED_TOKEN as usize * modifier_definition::STRIDE
            + modifier_definition::CATEGORY_MASK,
        1,
    );
    m.global(modifier_definition::TRADE_FEATURE_FLAGS_RVA, 1 << 4);
    m.global(BASE_SPEED_RVA, 200_000);
    m.global(MAXIMUM_TOTAL_RVA, 100_000);
    m.global(MINIMUM_INTEL_RVA, 0);
    m.global(MAXIMUM_INTEL_RVA, 100_000);
    m.global(INTEL_REDUCTION_RVA, 50_000);
    m
}

#[test]
fn hourly_transport_inputs_share_only_complete_current_observations() {
    use std::cell::RefCell;
    struct Observed<'a> {
        source: &'a Fixture,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            let index = self.reads.borrow().len();
            self.reads.borrow_mut().push((at, output.len()));
            if self.fault == Some(index) {
                output.fill(0xab);
                return false;
            }
            self.source.copy(at, output)
        }
    }
    let mut source = fixture();
    fn observed(source: &Fixture, fault: Option<usize>) -> Observed<'_> {
        Observed {
            source,
            reads: RefCell::new(vec![]),
            fault,
        }
    }
    let reference = observed(&source, None);
    let expected = movement_speed(&reference, 0, 0x1000, 1, 0xd000).unwrap();
    let first = observed(&source, None);
    assert_eq!(
        MovementSpeeds::new(8).get(&first, 0, 0x1000, 1, 0xd000),
        Ok(expected)
    );
    // The owner and enemy share this manager's accepted controls. Only their
    // repeated attempts disappear; every other operand keeps native order.
    let shared_controls = [
        0x1000 + cgame_state::STRATEGIC_NAVY_MANAGER,
        0x11000,
        0x11000 + cstrategic_navy_manager::NAVIES,
    ];
    let retained = |reads: &[(usize, usize)]| {
        let mut seen = std::collections::BTreeSet::new();
        reads
            .iter()
            .enumerate()
            .filter_map(|(index, &(at, width))| {
                (!shared_controls.contains(&at) || seen.insert((at, width))).then_some(index)
            })
            .collect::<Vec<_>>()
    };
    let direct_reads = reference.reads.borrow();
    let retained_indices = retained(&direct_reads);
    assert_eq!(
        *first.reads.borrow(),
        retained_indices
            .iter()
            .map(|&index| direct_reads[index])
            .collect::<Vec<_>>()
    );
    assert_eq!(direct_reads.len() - retained_indices.len(), 3);
    drop(direct_reads);
    for (fault, &direct_fault) in retained_indices.iter().enumerate() {
        let direct = observed(&source, Some(direct_fault));
        let shared = observed(&source, Some(fault));
        let mut memo = MovementSpeeds::new(8);
        let expected = movement_speed(&direct, 0, 0x1000, 1, 0xd000);
        assert!(expected.is_err());
        assert_eq!(
            memo.get(&shared, 0, 0x1000, 1, 0xd000),
            expected,
            "fault {fault}"
        );
        assert_eq!(
            *shared.reads.borrow(),
            retained(&direct.reads.borrow())
                .into_iter()
                .map(|index| direct.reads.borrow()[index])
                .collect::<Vec<_>>(),
            "fault order {fault}"
        );
        assert!(memo.owners.is_empty());
        assert!(memo.enemies.is_empty());
    }
    // A second region has different weather and naval operands but the
    // same owner's ordered enemy/reciprocal-intel dependencies.
    source.word(0xc200, 0xd100);
    source.word(0xd100, cstrategic_region::VTABLE_RVA as u64);
    source.dword(0xd100 + cstrategic_region::ID, 0);
    source.dword(0xd100 + cstrategic_region::RUNTIME_SLOT, 0);
    source.word(0xf000, sweather_per_region::VTABLE_RVA as u64);
    source.dword(0xf000 + sweather_per_region::ID, 0);
    source.word(0x14000, 100_000);
    let mut memo = MovementSpeeds::new(8);
    let shared = observed(&source, None);
    assert_eq!(memo.get(&shared, 0, 0x1000, 1, 0xd000), Ok(expected));
    shared.reads.borrow_mut().clear();
    assert_eq!(memo.get(&shared, 0, 0x1000, 1, 0xd000), Ok(expected));
    assert!(shared.reads.borrow().is_empty());
    let direct = observed(&source, None);
    let expected_second = movement_speed(&direct, 0, 0x1000, 1, 0xd100).unwrap();
    assert_ne!(expected_second, expected);
    assert_eq!(memo.get(&shared, 0, 0x1000, 1, 0xd100), Ok(expected_second));
    assert!(shared.reads.borrow().len() < direct.reads.borrow().len());
    assert!(
        !shared
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == 0xa000 + cdiplomacy_status::ENEMY_COUNTRIES)
    );
    assert!(shared.reads.borrow().iter().any(|&(at, _)| at == 0x14000));
    // Duplicate native enemies contribute once per occurrence. An arena
    // too small for the full roster must never publish a partial total.
    for count in [2, 9] {
        source.array(0xa000 + cdiplomacy_status::ENEMY_COUNTRIES, 0xc000, count);
        for index in 0..count {
            source.dword(0xc000 + index as usize * 4, 2);
        }
        for capacity in [1, 8] {
            let mut bounded = MovementSpeeds::new(capacity);
            for region in [0xd000, 0xd100] {
                assert_eq!(
                    bounded.get(&source, 0, 0x1000, 1, region),
                    movement_speed(&source, 0, 0x1000, 1, region)
                );
            }
            assert_eq!(bounded.enemies.len(), count as usize);
        }
    }
    // A changed enemy gate after the hourly reset must affect both regions.
    source.put(0xb000 + cdiplomacy_status::CAPITULATED, &[1]);
    memo.clear();
    assert_ne!(movement_speed(&source, 0, 0x1000, 1, 0xd000), Ok(expected));
    for region in [0xd000, 0xd100] {
        assert_eq!(
            memo.get(&source, 0, 0x1000, 1, region),
            movement_speed(&source, 0, 0x1000, 1, region)
        );
    }
    // Vacant/full storage keeps all source observations and exact values.
    let mut vacant = MovementSpeeds::new(0);
    let direct = observed(&source, None);
    let shared = observed(&source, None);
    assert_eq!(
        vacant.get(&shared, 0, 0x1000, 1, 0xd000),
        movement_speed(&direct, 0, 0x1000, 1, 0xd000)
    );
    assert_eq!(*shared.reads.borrow(), *direct.reads.borrow());
}

#[test]
fn transport_native_fraction_and_separate_rounding_are_preserved() {
    assert_eq!(fraction(1, 3), Ok(33_333));
    assert_eq!(fraction(-1, 3), Ok(-33_333));
    for numerator in [i64::MIN, i64::MAX, i64::MIN + 1] {
        assert_eq!(fraction(numerator, 7), Ok(0xffff_ffff));
    }
    assert_eq!(fraction(0, 0), Ok(0xffff_ffff));
    assert_eq!(multiply(i64::MAX, 2), 0);
    assert_eq!(
        reduction_fraction(50_000, 20_000, 100_000, 20_000, 0, 100_000, 50_000),
        Ok(35_000)
    );
    assert_eq!(
        reduction_fraction(50_000, 1, 100_000, 0, 0, 0, 77),
        Ok(50_000)
    );
    assert_eq!(
        reduction_fraction(50_000, 0, 100_000, 200_000, 0, 100_000, 50_000),
        Ok(0)
    );
    assert_eq!(
        reduction_fraction(200_000, 0, 100_000, -100_000, 0, 100_000, 50_000),
        Ok(100_000)
    );
}

#[test]
fn transport_speed_joins_reciprocal_intel_and_scales_raw_before_effective_metadata() {
    let mut m = fixture();
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(145_000));
    // A zero category suppresses only the temporary composed modifier.
    let metadata = 0x20000 + SPEED_TOKEN as usize * modifier_definition::STRIDE;
    m.dword(metadata + modifier_definition::CATEGORY_MASK, 0);
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(180_000));
    m.dword(metadata + modifier_definition::CATEGORY_MASK, 1);
    m.dword(
        metadata + modifier_definition::FLAGS,
        modifier_definition::CLAMP_MAXIMUM,
    );
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(200_000));
    // Scale the RAW definition first: 200000 * .35 = 70000, then
    // clamp. Clamping the definition before multiplication is wrong.
    m.word(0x17008, 200_000);
    m.word(0x10008, 0);
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(340_000));
    m.word(0x17008, (-50_000_i64) as u64);
    m.word(0x10008, (-10_000_i64) as u64);
    m.dword(metadata + modifier_definition::FLAGS, 0);
    // Disabled transport DLC must not touch otherwise-invalid penalty roots.
    m.global(modifier_definition::TRADE_FEATURE_FLAGS_RVA, 0);
    m.global(MODIFIER_SET_RVA, 0);
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(180_000));
}

#[test]
fn transport_static_definition_requires_derived_type_not_temporary_modifier() {
    let mut m = fixture();
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(145_000));
    for wrong_vtable in [0, crate::cmodifier::VTABLE_RVA as u64] {
        m.word(0x16000, wrong_vtable);
        assert_eq!(
            movement_speed(&m, 0, 0x1000, 1, 0xd000),
            Err("transport modifier identity mismatch"),
        );
    }
}

#[test]
fn transport_owner_navy_is_required_only_after_native_feature_gates() {
    for empty_enemies in [false, true] {
        for fault in 0..4 {
            let mut m = fixture();
            if empty_enemies {
                m.array(0xa000 + cdiplomacy_status::ENEMY_COUNTRIES, 0, 0);
            }
            assert_eq!(
                movement_speed(&m, 0, 0x1000, 1, 0xd000),
                Ok(if empty_enemies { 180_000 } else { 145_000 }),
            );
            match fault {
                0 => m.word(0x12008, 0),
                1 => m.dword(0x18000 + cstrategic_navy::OWNER, 2),
                2 => m.word(0x18000, 0),
                _ => m.word(0x18000 + cstrategic_navy::SECONDARY, 0),
            }
            assert!(movement_speed(&m, 0, 0x1000, 1, 0xd000).is_err());
            m.word(0x15000 + TRANSPORT_MODIFIER, 0);
            assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(180_000));
            m.word(0x15000 + TRANSPORT_MODIFIER, 0x16000);
            m.global(modifier_definition::TRADE_FEATURE_FLAGS_RVA, 0);
            assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(180_000));
        }
    }
}

#[test]
fn transport_faults_never_become_partial_enemy_aggregates() {
    for address in [0x13000, 0x9000, 0xb000, 0xd000, 0xe000] {
        let mut m = fixture();
        m.word(address, 0);
        assert!(
            movement_speed(&m, 0, 0x1000, 1, 0xd000).is_err(),
            "address {address:x}"
        );
    }
    let mut m = fixture();
    m.dword(0x13000 + cstrategic_navy::OWNER, 1);
    assert!(movement_speed(&m, 0, 0x1000, 1, 0xd000).is_err());
    let mut m = fixture();
    // Native out-of-range weather returns no weather modifier, not failure.
    m.array(0xe000 + cweather_manager::REGIONS, 0, 0);
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(165_000));
    // Native eligibility excludes capitulated enemies before navy access.
    m.bytes[0xb000 + cdiplomacy_status::CAPITULATED] = 1;
    m.word(0x13000, 0);
    assert_eq!(movement_speed(&m, 0, 0x1000, 1, 0xd000), Ok(200_000));
}

#[test]
fn observation_index_demand_excludes_allocator_spare_capacity() {
    let mut memo = MovementSpeeds::try_with_capacity(0).unwrap();
    memo.observations.reserve(30_000);
    memo.failures.reserve(30_000);
    let slots = observation_slots(5_001, 5_001).unwrap();
    for _ in 0..20 {
        memo.reserve_queries(5_001, 0).unwrap();
        assert_eq!(memo.observation_index.len(), slots);
    }
}
