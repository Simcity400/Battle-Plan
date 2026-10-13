use super::*;
use crate::{cstrategic_navy, memory::SliceMemory};
use std::collections::BTreeMap;

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

#[derive(Default)]
struct TransferFixture {
    bytes: BTreeMap<usize, u8>,
    calls: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl TransferFixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.bytes.insert(address + offset, *byte);
        }
    }
}

impl Memory for TransferFixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, output.len()));
        for (index, byte) in output.iter_mut().enumerate() {
            let Some(source) = self.bytes.get(&(address + index)) else {
                return false;
            };
            *byte = *source;
        }
        true
    }
}

#[test]
fn transfer_uses_the_original_selector_and_signed_status() {
    const TRANSFER: usize = 0x12000;
    const TRANSFER_HANDLE: u64 = (7u64 << 32) | 4713;
    let mut memory = TransferFixture::default();
    let mut resolves = 0;
    assert_eq!(
        transfer(&memory, 0, Err("original selector failed"), |_| panic!(
            "unreached registry"
        )),
        Err("original selector failed")
    );
    assert_eq!(
        transfer(&memory, 0, Ok(0), |_| panic!("null registry")),
        Ok(Transfer::Absent)
    );
    assert!(memory.calls.borrow().is_empty());
    memory.put(TRANSFER, &(VTABLE_RVA as u64).to_le_bytes());
    memory.put(TRANSFER + HANDLE, &TRANSFER_HANDLE.to_le_bytes());
    for status in [-7i32, 0] {
        memory.put(TRANSFER + ACTION_STATUS, &status.to_le_bytes());
        assert_eq!(
            transfer(&memory, 0, Ok(TRANSFER), |handle| {
                assert_eq!(handle, TRANSFER_HANDLE);
                resolves += 1;
                Ok(Some(TRANSFER))
            }),
            Ok(Transfer::Present {
                handle: TRANSFER_HANDLE,
                status: Ok(status),
            })
        );
    }
    assert_eq!(resolves, 2);
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|(address, _)| *address == TRANSFER + SOURCE_PROVINCE)
    );
}

#[test]
fn transfer_authenticated_identity_survives_independent_status_refusal() {
    const TRANSFER: usize = 0x12000;
    const TRANSFER_HANDLE: u64 = (7u64 << 32) | 4713;
    let mut memory = TransferFixture::default();
    memory.put(TRANSFER, &(VTABLE_RVA as u64).to_le_bytes());
    memory.put(TRANSFER + HANDLE, &TRANSFER_HANDLE.to_le_bytes());
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), |_| Ok(Some(TRANSFER))),
        Ok(Transfer::Present {
            handle: TRANSFER_HANDLE,
            status: Err("selected transport transfer status unreadable"),
        })
    );
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), |_| Ok(Some(TRANSFER + 1))),
        Err("selected transport transfer registry mismatch")
    );
}
