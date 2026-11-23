use super::*;
use engine_native_parts::{
    creference_registry as registry,
    modifier_definition,
    reference_bucket,
};
use std::{cell::RefCell, collections::BTreeMap};

const MODULE: usize = 0x140000000;
const GAME: usize = 0x1000;
const COUNTRY: usize = 0x10000;
const ACE_OWNER: usize = 0x20000;
const AIR: usize = 0x30000;
const POOL: usize = 0x40000;
const SUBTYPE: usize = 0x50000;
const WING: usize = 0x60000;
const REGION: usize = 0x70000;
const PROVINCE: usize = 0x90000;
const WEATHER: usize = 0xa0000;
const CLOCK: usize = 0xb0000;
const ACE: usize = 0xc0000;
const ASSIGNED: usize = 0xd0000;
const BASE: usize = 0xe0000;
const HOST: usize = 0xf0000;
const COSTS: usize = 0x600000;
const DEFINITIONS: usize = 0x610000;
const THRESHOLDS: usize = 0x620000;

#[derive(Default)]
struct Synthetic {
    bytes: BTreeMap<usize, u8>,
    calls: RefCell<Vec<(usize, usize)>>,
    next: usize,
}

impl Memory for Synthetic {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, output.len()));
        for (index, byte) in output.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

impl Synthetic {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes.extend(
            bytes
                .iter()
                .enumerate()
                .map(|(index, value)| (address + index, *value)),
        );
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn i64(&mut self, address: usize, value: i64) {
        self.put(address, &value.to_le_bytes());
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn i32(&mut self, address: usize, value: i32) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn object(&mut self, pointer: usize, rva: usize) {
        self.u64(pointer, (MODULE + rva) as u64);
    }
    fn reference(&mut self, handle: u64, object: usize) {
        let kind = handle as u32;
        assert!(kind < registry::LOW_ROOT_COUNT);
        let table = 0x300000 + kind as usize * 0x1000;
        let entries = table + 0x100;
        self.u64(
            MODULE + registry::LOW_ROOTS_RVA.0 + kind as usize * 8,
            table as u64,
        );
        self.u64(table + registry::ENTRIES, entries as u64);
        self.u32(table + registry::COUNT, 1);
        self.u32(table + registry::MASK, 7);
        self.put(table + registry::OVERFLOW_DISTANCE, &[5]);
        self.put(entries, &[0; 14 * reference_bucket::STRIDE]);
        self.put(
            entries + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE,
            &[255],
        );
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let entry = entries + slot * reference_bucket::STRIDE;
        self.put(entry + reference_bucket::DISTANCE, &[1]);
        self.u64(entry + reference_bucket::TYPE, handle);
        self.u64(entry + reference_bucket::OBJECT, object as u64);
    }
    fn modifier(&mut self, object: usize, offset: usize, key: u32, raw: i64) {
        let data = self.next;
        self.next += 0x100;
        self.array(object + offset, data, 1);
        self.u32(data, key);
        self.i64(data + 8, raw);
        self.u32(
            DEFINITIONS
                + key as usize * modifier_definition::STRIDE
                + modifier_definition::FEATURE_SELECTOR,
            0,
        );
        self.u32(
            DEFINITIONS + key as usize * modifier_definition::STRIDE + modifier_definition::FLAGS,
            0,
        );
    }
    fn times(&self, address: usize, length: usize) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|call| **call == (address, length))
            .count()
    }
}

fn fixture() -> Synthetic {
    let mut memory = Synthetic {
        next: 0x700000,
        ..Synthetic::default()
    };
    memory.array(GAME + cgame_state::COUNTRIES, 0x2000, 3);
    memory.array(GAME + cgame_state::COUNTRY_LINKS, 0x2100, 3);
    memory.u64(0x2000, 0);
    memory.u64(0x2008, COUNTRY as u64);
    memory.u64(0x2010, ACE_OWNER as u64);
    memory.u32(0x2100, 0);
    memory.u32(0x2104, 1);
    memory.u32(0x2108, 2);
    for (country, id) in [(COUNTRY, 1), (ACE_OWNER, 2)] {
        memory.object(country, ccountry::VTABLE_RVA.0);
        memory.u32(country + ccountry::ID, id);
        memory.array(country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0, 0);
    }
    memory.u32(AIR + cstrategic_air::COUNTRY, 1);
    memory.object(WING + cair_wing::MISSION, air_mission::VTABLE_RVA.0);
    memory.i32(WING + cair_wing::MISSION + air_mission::CURRENT_TYPE, 2);
    memory.i32(WING + MISSION_PLANES, 2);
    memory.i32(WING + cair_wing::PLANE_COUNT, 3);
    memory.i64(WING + cair_wing::BASE_FUEL, 100000);
    memory.u64(WING + cair_wing::ACE_REFERENCE, 0);
    memory.i32(WING + cair_wing::COUNTRY_OVERRIDE, 0);
    memory.put(WING + cair_wing::MISSION + air_mission::CONTEXT_MODE, &[2]);
    memory.u64(WING + cair_wing::MISSION + air_mission::REGION, 0);
    memory.array(MODULE + defines::MISSION_COSTS_RVA.0, COSTS, 19);
    for index in 0..19 {
        memory.i64(COSTS + index * 8, 100000);
    }
    memory.i64(COSTS + 8, 200000);
    memory.i64(COSTS + 18 * 8, 9000000);
    memory.i64(MODULE + defines::AIR_WEIGHT_RVA.0, 100000);
    memory.u64(
        MODULE + modifier_definition::TABLE_RVA.0,
        DEFINITIONS as u64,
    );
    memory.array(MODULE + defines::CAPACITY_THRESHOLDS_RVA.0, THRESHOLDS, 1);
    memory.i64(THRESHOLDS, 50000);
    memory
}

fn input(count: u32) -> PoolInput {
    PoolInput {
        pointer: POOL,
        owner: AIR,
        base_reference: (9_u64 << 32) | 65,
        definition: SUBTYPE,
        wing_count: count,
    }
}

fn capture(memory: &Synthetic, wings: &[usize]) -> Packet {
    let countries = ccountry::Reader::new(memory, MODULE, GAME).unwrap();
    let mut country_terms = vec![CountryTerms::default(); 3];
    let mut aces = vec![AceSlot::default(); 4];
    let mut forecasts = vec![ForecastSlot::default(); 4];
    let mut reader = Reader::new(
        memory,
        MODULE,
        GAME,
        &countries,
        &mut country_terms,
        &mut aces,
        &mut forecasts,
    );
    let mut pool = reader.begin_pool(input(wings.len() as u32));
    for wing in wings {
        reader.wing(&mut pool, *wing);
    }
    pool.finish()
}

fn capture_with_planes(memory: &Synthetic) -> (Packet, Result<i32, &'static str>) {
    let countries = ccountry::Reader::new(memory, MODULE, GAME).unwrap();
    let mut country_terms = vec![CountryTerms::default(); 3];
    let mut aces = vec![AceSlot::default(); 4];
    let mut forecasts = vec![ForecastSlot::default(); 4];
    let mut reader = Reader::new(
        memory,
        MODULE,
        GAME,
        &countries,
        &mut country_terms,
        &mut aces,
        &mut forecasts,
    );
    let mut pool = reader.begin_pool(input(1));
    let planes = reader.wing_with_planes(&mut pool, WING);
    (pool.finish(), planes)
}

#[test]
fn transport_plane_union_borrows_original_maximum_attempt_and_keeps_current_independent() {
    let mut memory = fixture();
    memory.bytes.remove(&(WING + cair_wing::PLANE_COUNT));
    let (fuel, planes) = capture_with_planes(&memory);
    assert!(planes.is_err());
    assert_eq!(fuel.current, Some(400000));
    assert_eq!(fuel.maximum, None);
    assert_eq!(
        memory.times(WING + cair_wing::PLANE_COUNT, 4),
        1,
        "the failed original maximum leaf must not be retried for transport"
    );
}

#[test]
fn transport_plane_union_borrows_failed_training_leaf_without_retrying_either_fuel_query() {
    let mut memory = fixture();
    training(&mut memory);
    memory.bytes.remove(&(WING + cair_wing::PLANE_COUNT));
    let (fuel, planes) = capture_with_planes(&memory);
    assert!(planes.is_err());
    assert_eq!((fuel.current, fuel.maximum), (None, None));
    assert_eq!(memory.times(WING + cair_wing::PLANE_COUNT, 4), 1);
}

#[test]
fn transport_plane_union_reads_only_an_originally_unreached_plane_leaf() {
    let mut memory = fixture();
    memory.i32(WING + cair_wing::MISSION + air_mission::CURRENT_TYPE, 0);
    memory.u32(WING + cair_wing::MISSION + air_mission::ENABLED_MASK, 0);
    memory
        .bytes
        .remove(&(MODULE + defines::MISSION_COSTS_RVA.0));
    let (fuel, planes) = capture_with_planes(&memory);
    assert_eq!(planes, Ok(3));
    assert_eq!((fuel.current, fuel.maximum), (Some(0), None));
    assert_eq!(memory.times(WING + cair_wing::PLANE_COUNT, 4), 1);
}

fn region(memory: &mut Synthetic, province_count: u32) {
    memory.put(WING + cair_wing::MISSION + air_mission::CONTEXT_MODE, &[1]);
    memory.u64(
        WING + cair_wing::MISSION + air_mission::REGION,
        REGION as u64,
    );
    memory.object(REGION, cstrategic_region::VTABLE_RVA.0);
    memory.object(
        REGION + cstrategic_region::SECONDARY,
        cstrategic_region::SECONDARY_VTABLE_RVA.0,
    );
    memory.u32(REGION + cstrategic_region::RUNTIME_SLOT, 0);
    memory.array(GAME + cgame_state::STRATEGIC_REGIONS, 0x80000, 1);
    memory.u64(0x80000, REGION as u64);
    memory.array(
        REGION + cstrategic_region::PROVINCES,
        0x81000,
        province_count,
    );
    for index in 0..province_count {
        memory.u64(0x81000 + index as usize * 8, PROVINCE as u64);
    }
    memory.i64(MODULE + defines::CONTEXT_THRESHOLD_RVA.0, 50000);
}

fn weather(memory: &mut Synthetic) {
    memory.u64(GAME + cgame_state::WEATHER_MANAGER, WEATHER as u64);
    memory.object(WEATHER, cweather_manager::VTABLE_RVA.0);
    memory.i64(WEATHER + FORECAST_COEFFICIENTS, 100000);
    memory.i64(WEATHER + FORECAST_COEFFICIENTS + 8, 0);
    memory.i64(WEATHER + FORECAST_COEFFICIENTS + 16, 0);
    memory.i64(MODULE + defines::CONTEXT_SUBTRACT_RVA.0, 0);
    memory.i64(MODULE + defines::CONTEXT_SCALE_RVA.0, 100000);
    memory.array(WEATHER + cweather_manager::PROVINCE_CLOCKS, CLOCK, 1);
    memory.array(GAME + cgame_state::PROVINCES, 0x82000, 1);
    memory.u64(0x82000, PROVINCE as u64);
    memory.object(PROVINCE, cprovince::VTABLE_RVA.0);
    memory.i32(PROVINCE + cprovince::ID, 0);
    for hour in 0..24 {
        memory.i64(CLOCK + hour * 24, if hour < 6 { 100000 } else { 0 });
        memory.i64(CLOCK + hour * 24 + 8, 0);
        memory.i64(CLOCK + hour * 24 + 16, 0);
    }
}

fn training(memory: &mut Synthetic) {
    memory.i32(WING + cair_wing::MISSION + air_mission::CURRENT_TYPE, 0);
    memory.u32(
        WING + cair_wing::MISSION + air_mission::ENABLED_MASK,
        air_mission::TRAINING_MASK,
    );
    memory.put(WING + cair_wing::ELIGIBILITY_VETO, &[0]);
    memory.i32(WING + cair_wing::ELIGIBILITY_COUNTER, 0);
    memory.i32(WING + cair_wing::ELIGIBILITY_LIMIT, 0);
    memory.u64(WING + cair_wing::BASE_OVERRIDE, 0);
    memory.u64(WING + cair_wing::MISSION + air_mission::WING, WING as u64);
    memory.i32(WING + cair_wing::MISSION + air_mission::CAPACITY_CLASS, 0);
    memory.reference(input(1).base_reference, BASE);
    memory.object(BASE, engine_native_parts::cair_base_layout::VTABLE_RVA.0);
    memory.u64(BASE + engine_native_parts::cair_base_layout::HANDLE, input(1).base_reference);
    memory.u64(BASE + cair_base::STATE, 0);
    let host_reference = (17_u64 << 32) | 51;
    memory.reference(host_reference, HOST);
    memory.object(HOST, cship::VTABLE_RVA.0);
    memory.u64(HOST + cship::HANDLE, host_reference);
    memory.u64(BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE, host_reference);
    memory.put(HOST + cship::FORMATION_FLAGS, &[0x80]);
    memory.u64(SUBTYPE + csub_unit_definition::AIR_FLAGS, 1 << 10);
    memory.i32(SUBTYPE + SUBTYPE_CAPACITIES + 12, 6);
}

#[test]
fn empty_pool_is_known_zero_without_any_fuel_or_forecast_queries() {
    let mut memory = fixture();
    memory
        .bytes
        .remove(&(MODULE + defines::MISSION_COSTS_RVA.0));
    let packet = capture(&memory, &[]);
    assert_eq!(
        packet,
        Packet {
            current: Some(0),
            maximum: Some(0),
            ..Packet::default()
        }
    );
    assert_eq!(memory.times(MODULE + defines::MISSION_COSTS_RVA.0, 16), 0);
}

#[test]
fn current_type_and_mission_aircraft_are_independent_of_saved_active() {
    let memory = fixture();
    assert_eq!(
        capture(&memory, &[WING]),
        Packet {
            current: Some(400000),
            maximum: Some(600000),
            ..Packet::default()
        }
    );
    assert_eq!(
        memory.times(WING + cair_wing::MISSION + air_mission::ACTIVE, 1),
        0
    );
    assert_eq!(memory.times(COSTS, MAXIMUM_MISSIONS * 8), 1);
}

#[test]
fn signed_planes_repeats_and_zero_initial_maximum_keep_native_arithmetic() {
    let mut memory = fixture();
    memory.i32(WING + cair_wing::PLANE_COUNT, -3);
    assert_eq!(capture(&memory, &[WING, WING]).maximum, Some(-1200000));
    for index in 0..MAXIMUM_MISSIONS {
        memory.i64(COSTS + index * 8, -100000);
    }
    assert_eq!(capture(&memory, &[WING]).maximum, Some(0));
    assert_eq!(maximum_term(i32::MAX, P, P, P), 30_280_923_962_904);
    assert_ne!(
        maximum_term(i32::MAX, P, P, P),
        i64::from(i32::MAX).wrapping_mul(P)
    );
    assert_eq!(modified_base(100001, 0, -1), 99999);
    assert_eq!(current_term(-100001, 99999, 100001), -99999);
}

#[test]
fn current_forecast_fault_preserves_maximum_and_mode_two_skips_forecasts() {
    let mut memory = fixture();
    region(&mut memory, 1);
    assert_eq!(capture(&memory, &[WING]).maximum, Some(600000));
    assert!(capture(&memory, &[WING]).current.is_none());
    memory.put(WING + cair_wing::MISSION + air_mission::CONTEXT_MODE, &[2]);
    memory.calls.borrow_mut().clear();
    assert_eq!(capture(&memory, &[WING]).current, Some(400000));
    assert_eq!(memory.times(REGION, 8), 0);
    assert_eq!(memory.times(GAME + cgame_state::WEATHER_MANAGER, 8), 0);
}

#[test]
fn complete_region_forecasts_preserve_repeated_provinces_and_cache_within_pass() {
    let mut memory = fixture();
    region(&mut memory, 2);
    weather(&mut memory);
    assert_eq!(capture(&memory, &[WING, WING]).current, Some(200000));
    assert_eq!(memory.times(CLOCK, FORECAST_VECTOR_BYTES), 2);
    assert_eq!(memory.times(WEATHER + FORECAST_COEFFICIENTS, 24), 1);
    for hour in 0..24 {
        memory.i64(CLOCK + hour * 24, 50000);
    }
    assert_eq!(capture(&memory, &[WING]).current, Some(0));
    memory.put(WING + cair_wing::MISSION + air_mission::CONTEXT_MODE, &[0]);
    assert_eq!(capture(&memory, &[WING]).current, Some(400000));
}

#[test]
fn empty_region_uses_native_unsigned_sentinel_without_weather_values() {
    let mut memory = fixture();
    region(&mut memory, 0);
    assert_eq!(capture(&memory, &[WING]).current, Some(400000));
    assert_eq!(memory.times(GAME + cgame_state::WEATHER_MANAGER, 8), 0);
    memory.put(WING + cair_wing::MISSION + air_mission::CONTEXT_MODE, &[0]);
    assert_eq!(capture(&memory, &[WING]).current, Some(0));
    assert_eq!(region_average(0, 0), SENTINEL);
    assert_eq!(province_forecast([100001, -2, 3], [P, P, P], 0, P), P);
}

#[test]
fn inactive_current_skips_missing_fuel_and_keeps_a_failed_maximum_independent() {
    let mut memory = fixture();
    memory.i32(WING + cair_wing::MISSION + air_mission::CURRENT_TYPE, 0);
    memory.u32(WING + cair_wing::MISSION + air_mission::ENABLED_MASK, 0);
    memory.bytes.remove(&(WING + cair_wing::BASE_FUEL));
    let packet = capture(&memory, &[WING]);
    assert_eq!(packet.current, Some(0));
    assert_eq!(packet.maximum, None);
    memory.bytes.remove(&COSTS);
    memory.calls.borrow_mut().clear();
    assert_eq!(capture(&memory, &[WING]).current, Some(0));
    assert_eq!(memory.times(WING + cair_wing::BASE_FUEL, 8), 0);
}

#[test]
fn active_training_gate_uses_base_capacity_and_authenticates_mission_backlink() {
    let mut memory = fixture();
    training(&mut memory);
    assert_eq!(capture(&memory, &[WING]).current, Some(300000));
    assert_eq!(memory.times(HOST + SHIP_CARRIER_CACHE + 1, 1), 0);
    memory.u64(
        WING + cair_wing::MISSION + air_mission::WING,
        (WING + 1) as u64,
    );
    assert_eq!(capture(&memory, &[WING]).current, None);
    assert_eq!(capture(&memory, &[WING]).maximum, Some(600000));
    assert!(capacity_gate(3, 0, P));
    assert!(!capacity_gate(3, 0, P + 1));
    assert!(!capacity_gate(3, -6, 0));
}

fn dirty_host(memory: &mut Synthetic) {
    memory.put(HOST + cship::FORMATION_FLAGS, &[0]);
    memory.put(HOST + SHIP_CARRIER_CACHE + 1, &[0]);
    memory.object(
        HOST + cship::EQUIPMENT_POOL,
        cequipment_variant_pool::VTABLE_RVA.0,
    );
    memory.array(
        HOST + cship::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        0x640000,
        2,
    );
    for index in 0..2 {
        let variant = 0x650000 + index * 0x1000;
        memory.u64(0x640000 + index * 16, variant as u64);
        memory.object(variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0);
        memory.u64(
            variant + cequipment_variant::TYPE,
            (21 + index as u64) << 32 | 70,
        );
        memory.array(
            variant + cequipment_variant::MODULES + 8,
            0x670000 + index * 0x1000,
            u32::from(index == 0),
        );
    }
    memory.u64(0x670008, 0x680000);
    memory.object(0x680000, cequipment_module::VTABLE_RVA.0);
    memory.put(0x680000 + MODULE_CARRIER_FLAG, &[1]);
    memory.i32(SUBTYPE + SUBTYPE_CAPACITIES + 8, 6);
}

#[test]
fn dirty_noncarrier_host_queries_modules_without_cache_writes_or_amount_filter() {
    let mut memory = fixture();
    training(&mut memory);
    dirty_host(&mut memory);
    assert_eq!(capture(&memory, &[WING]).current, Some(300000));
    assert_eq!(memory.times(HOST + SHIP_CARRIER_CACHE, 1), 0);
    assert_eq!(memory.times(0x651000, 16), 1);
    memory.bytes.remove(&0x651000);
    let packet = capture(&memory, &[WING]);
    assert_eq!(packet.current, None);
    assert_eq!(packet.maximum, Some(600000));
}

#[test]
fn valid_noncarrier_cache_preserves_its_stored_value_without_module_query() {
    let mut memory = fixture();
    training(&mut memory);
    memory.put(HOST + cship::FORMATION_FLAGS, &[0]);
    memory.put(HOST + SHIP_CARRIER_CACHE, &[1, 1]);
    memory.i32(SUBTYPE + SUBTYPE_CAPACITIES + 8, 6);
    assert_eq!(capture(&memory, &[WING]).current, Some(300000));
    assert_eq!(memory.times(HOST + cship::EQUIPMENT_POOL, 8), 0);
}

#[test]
fn effective_ace_uses_fresh_assigned_count_and_ace_owner_effectiveness() {
    let mut memory = fixture();
    let reference = (19_u64 << 32) | 7;
    let wing_reference = (23_u64 << 32) | 69;
    memory.reference(reference, ACE);
    memory.reference(wing_reference, ASSIGNED + engine_native_parts::cair_wing_layout::SECONDARY);
    memory.object(ACE, cace::VTABLE_RVA.0);
    memory.u64(ACE + cace::REFERENCE, reference);
    memory.put(ACE + cace::ALIVE, &[1]);
    memory.u64(ACE + cace::ASSIGNED_WING_REFERENCE, wing_reference);
    memory.u64(ACE + cace::OWNER_COUNTRY, ACE_OWNER as u64);
    memory.object(ASSIGNED, engine_native_parts::cair_wing_layout::VTABLE_RVA.0);
    memory.object(
        ASSIGNED + engine_native_parts::cair_wing_layout::SECONDARY,
        engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA.0,
    );
    memory.u64(ASSIGNED + engine_native_parts::cair_wing_layout::REFERENCE_TYPE, wing_reference);
    memory.i32(ASSIGNED + cair_wing::PLANE_COUNT, 5);
    memory.i64(MODULE + defines::ACE_COUNT_RVA.0, 2000000);
    memory.i64(MODULE + defines::ACE_CAP_RVA.0, 1000000);
    memory.modifier(ACE, cace::MODIFIERS, FUEL_KEY, 10000);
    memory.modifier(ACE_OWNER, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, ACE_KEY, 50000);
    memory.u64(WING + cair_wing::ACE_REFERENCE, reference);
    assert_eq!(
        capture(&memory, &[WING, WING]),
        Packet {
            current: Some(1280000),
            maximum: Some(1920000),
            ..Packet::default()
        }
    );
    assert_eq!(memory.times(ASSIGNED + cair_wing::PLANE_COUNT, 4), 1);
    memory.i32(ASSIGNED + cair_wing::PLANE_COUNT, 0);
    memory.bytes.remove(&(MODULE + defines::ACE_COUNT_RVA.0));
    assert_eq!(capture(&memory, &[WING]).maximum, Some(690000));
    memory.u64(ASSIGNED + engine_native_parts::cair_wing_layout::SECONDARY, 0);
    assert_eq!(capture(&memory, &[WING]).maximum, None);
    memory.bytes.remove(&(ASSIGNED + engine_native_parts::cair_wing_layout::SECONDARY));
    assert_eq!(capture(&memory, &[WING]).current, None);
}

#[test]
fn invalid_current_masks_and_capacity_refusals_do_not_erase_known_maximum() {
    for current in [-1, 3] {
        let mut memory = fixture();
        memory.i32(
            WING + cair_wing::MISSION + air_mission::CURRENT_TYPE,
            current,
        );
        let packet = capture(&memory, &[WING]);
        assert_eq!(packet.current, None);
        assert_eq!(packet.maximum, Some(600000));
    }
    let mut memory = fixture();
    training(&mut memory);
    memory.i32(WING + cair_wing::MISSION + air_mission::CAPACITY_CLASS, -1);
    assert_eq!(capture(&memory, &[WING]).current, None);
    assert_eq!(capture(&memory, &[WING]).maximum, Some(600000));
}

#[test]
fn growing_memo_preserves_failed_and_successful_observations() {
    const KEYS: u64 = 12;
    let mut aces = Vec::new();
    let mut slots = Buffer::Growing(&mut aces);
    grow_memo(&mut slots, 2 * KEYS as usize, |slot: AceSlot| {
        slot.value.map(|_| ace_hash(slot.reference))
    })
    .unwrap();
    for reference in 1..=KEYS {
        let mut index = ace_hash(reference) % slots.len();
        while slots[index].value.is_some() {
            index = (index + 1) % slots.len();
        }
        slots[index] = AceSlot {
            reference,
            value: Some(if reference % 2 == 0 {
                Ok(reference as i64)
            } else {
                Err("first native fault")
            }),
        };
    }
    grow_memo(&mut slots, 6 * KEYS as usize, |slot| {
        slot.value.map(|_| ace_hash(slot.reference))
    })
    .unwrap();
    for reference in 1..=KEYS {
        let mut index = ace_hash(reference) % slots.len();
        while slots[index].reference != reference {
            index = (index + 1) % slots.len();
        }
        assert_eq!(
            slots[index].value,
            Some(if reference % 2 == 0 {
                Ok(reference as i64)
            } else {
                Err("first native fault")
            })
        );
    }
    assert_eq!(
        slots.iter().filter(|slot| slot.value.is_some()).count(),
        KEYS as usize
    );
}
