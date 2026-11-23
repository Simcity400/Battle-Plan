use super::*;
use engine_native_memory::memory::SliceMemory;

struct CarrierMemory<'a> {
    bytes: &'a [u8],
    fault: Option<usize>,
    globals: std::collections::BTreeMap<usize, u8>,
    reads: std::cell::RefCell<Vec<usize>>,
}

impl Memory for CarrierMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        if self.fault.is_some_and(|fault| {
            address <= fault
                && address
                    .checked_add(destination.len())
                    .is_some_and(|end| fault < end)
        }) {
            return false;
        }
        if (0..destination.len()).all(|index| self.globals.contains_key(&(address + index))) {
            for (index, byte) in destination.iter_mut().enumerate() {
                *byte = self.globals[&(address + index)];
            }
            return true;
        }
        if address
            == IMAGE
                + engine_native_parts::creference_registry::LOW_ROOTS_RVA.0
                + engine_native::cship::HANDLE_TYPE as usize * 8
            && destination.len() == 8
        {
            destination.fill(0);
            return true;
        }
        SliceMemory {
            base: GAME,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

struct CarrierWalkFixture {
    main: RegistrationWalk,
    rows: Vec<RegistrationRow>,
    wings: Vec<u64>,
    bases: Vec<CarrierBase>,
    registrations: Vec<CarrierRegistration>,
    pools: Vec<CarrierPool>,
    demand: CarrierDemand,
    lengths: CarrierLengths,
    optional_reads: Vec<usize>,
}

const CARRIER_PRIMARY_A: usize = 0x9000;
const CARRIER_PRIMARY_B: usize = 0xa000;
const CARRIER_OWNER_A: usize = 0x11000;
const CARRIER_OWNER_B: usize = 0x12000;
const CARRIER_CLONE: usize = 0x14000;
const CARRIER_GLOBAL: usize = 0x15000;
const CARRIER_NULL: usize = 0x16000;
const CARRIER_DATABASE: usize = 0x17000;
const CARRIER_DEFINITIONS: usize = 0x18000;
const CARRIER_COSTS: usize = 0x19000;
const CARRIER_HANDLE: u64 = (9_u64 << 32) | TYPE as u64;

fn carrier_globals() -> std::collections::BTreeMap<usize, u8> {
    let mut bytes = std::collections::BTreeMap::new();
    for (pointer, value) in [
        (
            IMAGE + engine_native_parts::csub_unit_definition::DATABASE_RVA.0,
            CARRIER_DATABASE as u64,
        ),
        (
            IMAGE + engine_native_parts::csub_unit_definition::NULL_OBJECT_RVA.0,
            CARRIER_NULL as u64,
        ),
        (
            IMAGE + engine_native_parts::air_fuel_defines::MISSION_COSTS_RVA.0,
            CARRIER_COSTS as u64,
        ),
        (IMAGE + engine_native_parts::air_fuel_defines::AIR_WEIGHT_RVA.0, 100000),
    ] {
        bytes.extend(
            value
                .to_le_bytes()
                .into_iter()
                .enumerate()
                .map(|(index, byte)| (pointer + index, byte)),
        );
    }
    for (pointer, value) in [
        (
            IMAGE + engine_native_parts::air_fuel_defines::MISSION_COSTS_RVA.0 + 8,
            19_u32,
        ),
        (
            IMAGE + engine_native_parts::air_fuel_defines::MISSION_COSTS_RVA.0 + 12,
            19_u32,
        ),
    ] {
        bytes.extend(
            value
                .to_le_bytes()
                .into_iter()
                .enumerate()
                .map(|(index, byte)| (pointer + index, byte)),
        );
    }
    bytes
}

fn carrier_definition(bytes: &mut [u8], pointer: usize, real: bool, token: u32) {
    let definition = if real {
        engine_native_parts::csub_unit_definition::VTABLE_RVA.0
    } else {
        engine_native_parts::csub_unit_definition::NULL_VTABLE_RVA.0
    };
    qword(bytes, pointer, (IMAGE + definition) as u64);
    dword(bytes, pointer + engine_native_parts::csub_unit_definition::TOKEN, token);
    bytes[pointer + engine_native_parts::csub_unit_definition::IS_REAL - GAME] = u8::from(real);
}

fn host_registration_fixture() -> Vec<u8> {
    let mut bytes = registration_fixture();
    bytes.resize(0x20000, 0);
    let header: [u8; 16] = bytes
        [STATE_BASE + COUNTRY_CONTAINERS - GAME..STATE_BASE + COUNTRY_CONTAINERS - GAME + 16]
        .try_into()
        .unwrap();
    bytes[HOST_BASE + COUNTRY_CONTAINERS - GAME..HOST_BASE + COUNTRY_CONTAINERS - GAME + 16]
        .copy_from_slice(&header);
    bytes[STATE_BASE + COUNTRY_CONTAINERS - GAME..STATE_BASE + COUNTRY_CONTAINERS - GAME + 16]
        .fill(0);
    const DENSE: usize = 0x2b00;
    array(&mut bytes, HOST_BASE + REGISTERED_CONTAINERS, DENSE, 2, 2);
    qword(&mut bytes, DENSE, CONTAINER_A as u64);
    qword(&mut bytes, DENSE + 8, CONTAINER_B as u64);
    qword(
        &mut bytes,
        CONTAINER_A + ccountry_air_container::FUEL_DEMAND_RAW,
        (-7_i64) as u64,
    );
    qword(
        &mut bytes,
        CONTAINER_A + ccountry_air_container::FUEL_ALLOCATED_RAW,
        11,
    );
    qword(
        &mut bytes,
        CONTAINER_B + ccountry_air_container::FUEL_DEMAND_RAW,
        23,
    );
    qword(
        &mut bytes,
        CONTAINER_B + ccountry_air_container::FUEL_ALLOCATED_RAW,
        29,
    );
    array(
        &mut bytes,
        CARRIER_DATABASE + engine_native_parts::csub_unit_definition::DATABASE_VECTOR,
        CARRIER_DEFINITIONS,
        2,
        2,
    );
    qword(&mut bytes, CARRIER_DEFINITIONS, CARRIER_NULL as u64);
    qword(&mut bytes, CARRIER_DEFINITIONS + 8, CARRIER_GLOBAL as u64);
    carrier_definition(&mut bytes, CARRIER_CLONE, true, 987);
    carrier_definition(&mut bytes, CARRIER_GLOBAL, true, 987);
    carrier_definition(
        &mut bytes,
        CARRIER_NULL,
        false,
        engine_native_parts::csub_unit_definition::NULL_TOKEN,
    );
    dword(
        &mut bytes,
        CARRIER_CLONE + engine_native_parts::csub_unit_definition::DATABASE_INDEX,
        1,
    );
    dword(&mut bytes, CARRIER_CLONE + 0x60, i32::MIN as u32);
    dword(&mut bytes, CARRIER_NULL + 0x60, (-17_i32) as u32);
    for (pool, owner, country, definition) in [
        (POOL_A, CARRIER_OWNER_A, 0_u32, CARRIER_CLONE),
        (POOL_B, CARRIER_OWNER_B, 2, CARRIER_NULL),
    ] {
        qword(
            &mut bytes,
            pool + cair_wing_pool::AIR_BASE_REFERENCE,
            CARRIER_HANDLE,
        );
        qword(&mut bytes, pool + cair_wing_pool::OWNER, owner as u64);
        qword(
            &mut bytes,
            pool + cair_wing_pool::DEFINITION,
            definition as u64,
        );
        qword(
            &mut bytes,
            owner,
            (IMAGE + crate::cstrategic_air::VTABLE_RVA.0) as u64,
        );
        dword(&mut bytes, owner + crate::cstrategic_air::COUNTRY, country);
    }
    array(
        &mut bytes,
        POOL_A + cair_wing_pool::WINGS,
        WING_TABLE_A,
        3,
        3,
    );
    for (index, pointer) in [CARRIER_PRIMARY_A, CARRIER_PRIMARY_A, CARRIER_PRIMARY_B]
        .into_iter()
        .enumerate()
    {
        qword(&mut bytes, WING_TABLE_A + index * 8, pointer as u64);
    }
    for (pointer, id, planes, mission_planes) in [
        (CARRIER_PRIMARY_A, 11_u64, 3_u32, 2_u32),
        (CARRIER_PRIMARY_B, 12, 1, 1),
    ] {
        wing(
            &mut bytes,
            pointer,
            (id << 32) | cair_wing::TYPE as u64,
            POOL_A,
        );
        qword(
            &mut bytes,
            pointer + cair_wing::MISSION,
            (IMAGE + crate::air_mission::VTABLE_RVA.0) as u64,
        );
        dword(
            &mut bytes,
            pointer + cair_wing::MISSION + crate::air_mission::CURRENT_TYPE,
            2,
        );
        dword(&mut bytes, pointer + 0x170, mission_planes);
        dword(&mut bytes, pointer + cair_wing::PLANE_COUNT, planes);
        qword(&mut bytes, pointer + cair_wing::BASE_FUEL, 100000);
        qword(&mut bytes, pointer + cair_wing::ACE_REFERENCE, 0);
        dword(&mut bytes, pointer + cair_wing::COUNTRY_OVERRIDE, 5);
        bytes[pointer + cair_wing::MISSION + crate::air_mission::CONTEXT_MODE - GAME] = 2;
        qword(
            &mut bytes,
            pointer + cair_wing::MISSION + crate::air_mission::REGION,
            0,
        );
    }
    for country in [COUNTRY_A, COUNTRY_B] {
        array(
            &mut bytes,
            country + ccountry::MODIFIER_ACCUMULATOR,
            0,
            0,
            0,
        );
    }
    for index in 0..19 {
        qword(
            &mut bytes,
            CARRIER_COSTS + index * 8,
            if index == 1 {
                200000
            } else if index == 18 {
                9000000
            } else {
                100000
            },
        );
    }
    bytes
}

fn carrier_walk_fixture(
    bytes: &[u8],
    fault: Option<usize>,
    indexed_len: usize,
    main_row_capacity: usize,
) -> CarrierWalkFixture {
    carrier_walk_fixture_with_transport(bytes, fault, indexed_len, main_row_capacity, Ok(()))
}

fn carrier_walk_fixture_with_transport(
    bytes: &[u8],
    fault: Option<usize>,
    indexed_len: usize,
    main_row_capacity: usize,
    transport: Result<(), &'static str>,
) -> CarrierWalkFixture {
    carrier_walk_fixture_with_storage(
        bytes,
        fault,
        indexed_len,
        main_row_capacity,
        transport,
        CarrierDemand {
            country_terms: 3,
            aces: 8,
            forecasts: 8,
            ..Default::default()
        },
    )
}

fn carrier_walk_fixture_with_storage(
    bytes: &[u8],
    fault: Option<usize>,
    indexed_len: usize,
    main_row_capacity: usize,
    transport: Result<(), &'static str>,
    storage: CarrierDemand,
) -> CarrierWalkFixture {
    let memory = SliceMemory { base: GAME, bytes };
    let optional = CarrierMemory {
        bytes,
        fault,
        globals: carrier_globals(),
        reads: std::cell::RefCell::default(),
    };
    let countries = ccountry::Reader::new(&optional, IMAGE, GAME).unwrap();
    let mut source = vec![Row::default(); 4];
    let source_walk = walk(&memory, IMAGE, GAME, &mut source);
    let source_len = match source_walk {
        Outcome::Rows { len } => len,
        _ => 0,
    };
    let mut rows = vec![RegistrationRow::default(); main_row_capacity];
    let mut wings = vec![0; 8];
    let mut resolved = [None; 10];
    let mut bases = vec![CarrierBase::default(); 4];
    let mut registrations = vec![CarrierRegistration::default(); 4];
    let mut pools = vec![CarrierPool::default(); 4];
    let mut dense = vec![CountryOccurrence::default(); 8];
    let mut names = vec![0; 16];
    let mut indexed = vec![None; indexed_len];
    let mut optional_resolved = vec![None; 10];
    let mut country_terms = vec![CarrierCountryTerms::default(); storage.country_terms];
    let mut aces = vec![CarrierAceSlot::default(); storage.aces];
    let mut forecasts = vec![CarrierForecastSlot::default(); storage.forecasts];
    let (main, demand, lengths) = {
        let mut carrier = CarrierCapture::new(
            &optional,
            IMAGE,
            GAME,
            &countries,
            CarrierBuffers {
                bases: &mut bases,
                registrations: &mut registrations,
                pools: &mut pools,
                countries: &mut dense,
                names: &mut names,
                indexed: &mut indexed,
                resolved: &mut optional_resolved,
                country_terms: &mut country_terms,
                aces: &mut aces,
                forecasts: &mut forecasts,
            },
        )
        .with_transport_planes(transport);
        let main = walk_registrations_with_carrier(
            &memory,
            IMAGE,
            GAME,
            RegistrationBuffers {
                registrations: &mut rows,
                wings: &mut wings,
                resolved: &mut resolved,
            },
            &source[..source_len],
            &mut carrier,
        );
        (main, carrier.demand(), carrier.lengths())
    };
    let optional_reads = optional.reads.borrow().clone();
    CarrierWalkFixture {
        main,
        rows,
        wings,
        bases,
        registrations,
        pools,
        demand,
        lengths,
        optional_reads,
    }
}

#[test]
fn carrier_fuel_arrives_through_the_container_span_and_an_optional_fault_preserves_membership() {
    let bytes = host_registration_fixture();
    let original = registered(&bytes, 4, 8);
    let memory = SliceMemory {
        base: GAME,
        bytes: &bytes,
    };
    let separate = |container: usize| {
        (
            read::<i64>(&memory, container + ccountry_air_container::FUEL_DEMAND_RAW),
            read::<i64>(
                &memory,
                container + ccountry_air_container::FUEL_ALLOCATED_RAW,
            ),
        )
    };
    assert_eq!(separate(CONTAINER_A), (Some(-7), Some(11)));
    assert_eq!(separate(CONTAINER_B), (Some(23), Some(29)));
    let observed = carrier_walk_fixture(
        &bytes,
        Some(CONTAINER_A + ccountry_air_container::FUEL_DEMAND_RAW),
        4,
        4,
    );
    assert_eq!(
        (observed.main, &observed.rows, &observed.wings),
        (original.0, &original.1, &original.2)
    );
    assert_eq!(observed.registrations[0].fuel.demand, Ok(-7));
    assert_eq!(observed.registrations[0].fuel.allocated, Ok(11));
    assert_eq!(observed.registrations[1].country, 9);
    assert_eq!(observed.registrations[1].fuel.demand, Ok(23));
    assert_eq!(observed.registrations[1].fuel.allocated, Ok(29));
    for container in [CONTAINER_A, CONTAINER_B] {
        let words = container + ccountry_air_container::FUEL_DEMAND_RAW
            ..container + ccountry_air_container::FUEL_ALLOCATED_RAW + 8;
        assert!(
            !observed.optional_reads.iter().any(|at| words.contains(at)),
            "the observer reread a fuel word"
        );
    }
    assert_eq!(observed.bases[0].countries, Ok((0, 2)));
    assert_eq!(observed.bases[0].host_name, Ok(Some((0, 0))));
    assert_eq!(observed.registrations[0].pools, Ok((0, 1)));
    assert_eq!(observed.registrations[1].pools, Ok((1, 1)));
    assert_eq!(observed.rows[1].wings_len, 0);
    assert_eq!(observed.lengths.pools, 2);
    assert_eq!(
        observed.pools[0],
        CarrierPool {
            type_index: i32::MIN,
            subunit_index: 1,
            wings_start: 0,
            wings_len: 3,
            planes_raw: Some(7),
            fuel: CarrierPoolFuel {
                current: Some(1000000),
                maximum: Some(1400000),
                ..CarrierPoolFuel::default()
            }
        }
    );
    assert_eq!(
        observed.pools[1],
        CarrierPool {
            type_index: -17,
            subunit_index: 0,
            wings_start: 0,
            wings_len: 0,
            planes_raw: Some(0),
            fuel: CarrierPoolFuel {
                current: Some(0),
                maximum: Some(0),
                ..CarrierPoolFuel::default()
            }
        }
    );
    assert_eq!(
        observed.rows[1].wings_start, 3,
        "pool suffix start stays registration-relative"
    );
    assert_eq!(
        (
            observed.demand.pools,
            observed.demand.aces,
            observed.demand.forecasts
        ),
        (2, 6, 6)
    );
}

#[test]
fn original_carrier_plane_sum_wraps_repeats_and_keeps_empty_pool_zero() {
    let mut bytes = host_registration_fixture();
    dword(
        &mut bytes,
        CARRIER_PRIMARY_A + cair_wing::PLANE_COUNT,
        i32::MAX as u32,
    );
    let observed = carrier_walk_fixture(&bytes, None, 4, 4);
    assert_eq!(
        observed.pools[0].planes_raw,
        Some(-1),
        "MAX + MAX + 1 wraps as signed i32"
    );
    assert_eq!(observed.pools[1].planes_raw, Some(0));
    assert_eq!(
        observed.wings[..3],
        [
            (11_u64 << 32) | cair_wing::TYPE as u64,
            (11_u64 << 32) | cair_wing::TYPE as u64,
            (12_u64 << 32) | cair_wing::TYPE as u64,
        ]
    );
}

#[test]
fn failed_original_carrier_plane_leaf_preserves_membership_and_current_fuel() {
    let bytes = host_registration_fixture();
    let original = registered(&bytes, 4, 8);
    let observed = carrier_walk_fixture(
        &bytes,
        Some(CARRIER_PRIMARY_A + cair_wing::PLANE_COUNT),
        4,
        4,
    );
    assert_eq!(
        (observed.main, &observed.rows, &observed.wings),
        (original.0, &original.1, &original.2)
    );
    assert_eq!(observed.pools[0].planes_raw, None);
    assert_eq!(observed.pools[0].fuel.current, Some(1000000));
    assert_eq!(observed.pools[0].fuel.maximum, None);
    assert_eq!(observed.pools[1].planes_raw, Some(0));
    assert_eq!(
        observed
            .optional_reads
            .iter()
            .filter(|address| **address == CARRIER_PRIMARY_A + cair_wing::PLANE_COUNT)
            .count(),
        2,
        "each original occurrence attempts once; transport adds no retry"
    );
}

#[test]
fn denied_transport_carrier_demand_keeps_legacy_fuel_and_never_reads_unreached_planes() {
    let bytes = host_registration_fixture();
    let fault = Some(IMAGE + engine_native_parts::air_fuel_defines::MISSION_COSTS_RVA.0);
    let denied = carrier_walk_fixture_with_transport(
        &bytes,
        fault,
        4,
        4,
        Err("transport callback fixture denied"),
    );
    let admitted = carrier_walk_fixture_with_transport(&bytes, fault, 4, 4, Ok(()));
    assert_eq!(
        (denied.main, &denied.rows, &denied.wings),
        (admitted.main, &admitted.rows, &admitted.wings)
    );
    for index in 0..2 {
        assert_eq!(denied.pools[index].fuel, admitted.pools[index].fuel);
        assert_eq!(denied.pools[index].planes_raw, None);
    }
    assert_eq!(admitted.pools[0].planes_raw, Some(7));
    assert_eq!(admitted.pools[1].planes_raw, Some(0));
    for pointer in [CARRIER_PRIMARY_A, CARRIER_PRIMARY_B] {
        assert!(
            !denied
                .optional_reads
                .contains(&(pointer + cair_wing::PLANE_COUNT)),
            "denied transport cannot add an originally unreached leaf"
        );
        assert!(
            admitted
                .optional_reads
                .contains(&(pointer + cair_wing::PLANE_COUNT))
        );
    }
}

#[test]
fn carrier_name_survives_original_indexed_header_failure() {
    let mut bytes = host_registration_fixture();
    dword(&mut bytes, HOST_BASE + COUNTRY_CONTAINERS_COUNT, 5);
    let original = registered(&bytes, 4, 8);
    let observed = carrier_walk_fixture(&bytes, None, 4, 4);
    assert_eq!(observed.main, original.0);
    assert!(matches!(observed.main.outcome, Outcome::Unavailable { .. }));
    assert_eq!(observed.bases[0].host_name, Ok(Some((0, 0))));
    assert!(observed.bases[0].countries.is_err());
    assert_eq!(observed.lengths.registrations, 0);
    assert!(
        !observed
            .optional_reads
            .contains(&(HOST_BASE + REGISTERED_CONTAINERS)),
        "an original registration fault never reaches the dense sizing header"
    );
}

#[test]
fn carrier_dense_header_fault_names_the_inventory_field_and_keeps_scalars() {
    let bytes = host_registration_fixture();
    let observed = carrier_walk_fixture(&bytes, Some(HOST_BASE + REGISTERED_CONTAINERS), 0, 4);
    assert_eq!(observed.main.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        observed.bases[0].countries,
        Err("carrier inventory field unreadable or address overflow")
    );
    assert_eq!(observed.demand.failure, None);
    assert_eq!(observed.bases[0].host_name, Ok(Some((0, 0))));
    assert_eq!(observed.registrations[0].fuel.demand, Ok(-7));
    assert_eq!(observed.registrations[0].fuel.allocated, Ok(11));
    assert_eq!(observed.lengths.countries, 0);
    assert!(
        !observed
            .optional_reads
            .iter()
            .any(|address| (0x2b00..0x2b10).contains(address))
    );
}

#[test]
fn carrier_original_truncation_retains_complete_dense_order_but_no_dependent_pool_span() {
    let bytes = host_registration_fixture();
    let full = carrier_walk_fixture(&bytes, None, 4, 4);
    assert_eq!(full.registrations[0].pools, Ok((0, 1)));
    assert_eq!(full.pools[0].fuel.current, Some(1000000));
    assert_eq!(full.pools[0].fuel.maximum, Some(1400000));
    assert_eq!(full.lengths.pools, 2);
    let original = registered(&bytes, 0, 8);
    let observed = carrier_walk_fixture(&bytes, None, 4, 0);
    assert_eq!(observed.main, original.0);
    assert_eq!(
        observed.main.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!(observed.bases[0].countries, Ok((0, 2)));
    assert_eq!(
        observed.registrations[0].pools,
        Err("carrier registration membership not admitted")
    );
    assert_eq!(observed.registrations[0].fuel.demand, Ok(-7));
    assert_eq!(
        (observed.demand.registrations, observed.lengths.pools),
        (2, 0)
    );
}

#[test]
fn carrier_single_and_repeated_wing_callbacks_preserve_numeric_fuel_and_partial_queries() {
    let mut bytes = host_registration_fixture();
    dword(&mut bytes, POOL_A + cair_wing_pool::WINGS_COUNT, 1);
    let single = carrier_walk_fixture(&bytes, None, 4, 4);
    assert_eq!(
        single.pools[0].fuel,
        CarrierPoolFuel {
            current: Some(400000),
            maximum: Some(600000),
            ..CarrierPoolFuel::default()
        }
    );
    assert_eq!(single.pools[0].wings_len, 1);
    dword(&mut bytes, POOL_A + cair_wing_pool::WINGS_COUNT, 3);
    let partial = carrier_walk_fixture(
        &bytes,
        Some(CARRIER_PRIMARY_A + cair_wing::MISSION + crate::air_mission::REGION),
        4,
        4,
    );
    assert_eq!(partial.main.outcome, Outcome::Rows { len: 2 });
    assert_eq!(partial.registrations[0].pools, Ok((0, 1)));
    assert_eq!(partial.pools[0].fuel.current, None);
    assert!(partial.pools[0].fuel.current_failure.is_some());
    assert_eq!(partial.pools[0].fuel.maximum, Some(1400000));
    assert_eq!(partial.pools[0].fuel.maximum_failure, None);
    assert_eq!(partial.pools[1].fuel.current, Some(0));
}

#[test]
fn attach_carrier_hint_preserves_first_pass_fuel_with_each_transport_admission() {
    let bytes = host_registration_fixture();
    let memory = SliceMemory {
        base: GAME,
        bytes: &bytes,
    };
    let hint = carrier_demand(&memory, IMAGE, GAME).unwrap();
    assert_eq!(hint.country_terms, 3);
    assert_eq!((hint.aces, hint.forecasts), (6, 6));
    assert_eq!((hint.registrations, hint.pools), (2, 2));
    for admission in [Ok(()), Err("transport admission refused")] {
        let first = carrier_walk_fixture_with_storage(
            &bytes,
            None,
            hint.indexed,
            hint.registrations,
            admission,
            hint,
        );
        assert_eq!(first.main.outcome, Outcome::Rows { len: 2 });
        assert_eq!(first.registrations[0].pools, Ok((0, 1)));
        assert_eq!(first.pools[0].wings_len, 3);
        assert_eq!(
            (first.pools[0].fuel.current, first.pools[0].fuel.maximum),
            (Some(1000000), Some(1400000))
        );
        assert_eq!(first.pools[0].planes_raw, admission.ok().map(|()| 7));
        let fault = carrier_walk_fixture_with_storage(
            &bytes,
            Some(CARRIER_PRIMARY_A + cair_wing::PLANE_COUNT),
            hint.indexed,
            hint.registrations,
            admission,
            hint,
        );
        assert_eq!(fault.pools[0].fuel.current, Some(1000000));
        assert_eq!(fault.pools[0].fuel.maximum, None);
    }
}

#[test]
fn carrier_host_name_hint_uses_its_own_fresh_resolver_and_refuses_state_sources() {
    let bytes = host_registration_fixture();
    let memory = CarrierMemory {
        bytes: &bytes,
        fault: None,
        globals: carrier_globals(),
        reads: std::cell::RefCell::default(),
    };
    let source = Row {
        id: CARRIER_HANDLE,
        host: (123_u64 << 32) | engine_native::cship::HANDLE_TYPE as u64,
        ..Row::default()
    };
    assert_eq!(carrier_host_name_demand(&memory, IMAGE, &source), Ok(0));
    assert!(
        carrier_host_name_demand(
            &memory,
            IMAGE,
            &Row {
                state: Some(1),
                ..source
            }
        )
        .is_err()
    );
}

const IMAGE: usize = 0x140000000;
const GAME: usize = 0x1000;
const MANAGER: usize = 0x2000;
const STATE_TABLE: usize = 0x2400;
const HOST_TABLE: usize = 0x2500;
const STATE_BASE: usize = 0x3000;
const HOST_BASE: usize = 0x3200;
const STATE_OBJECT: usize = 0x4000;
const PROVINCE_OBJECT: usize = 0x5000;

fn qword(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at - GAME..at - GAME + 8].copy_from_slice(&value.to_le_bytes());
}

fn dword(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at - GAME..at - GAME + 4].copy_from_slice(&value.to_le_bytes());
}

fn array(bytes: &mut [u8], at: usize, data: usize, capacity: u32, count: u32) {
    qword(bytes, at, data as u64);
    dword(bytes, at + 8, capacity);
    dword(bytes, at + 12, count);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x5000];
    qword(
        &mut bytes,
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    qword(
        &mut bytes,
        MANAGER,
        (IMAGE + cstrategic_air_manager::VTABLE_RVA.0) as u64,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATE_AIR_BASES,
        STATE_TABLE,
        4,
        2,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOST_TABLE,
        2,
        1,
    );
    qword(&mut bytes, STATE_TABLE + 8, STATE_BASE as u64);
    qword(&mut bytes, HOST_TABLE, HOST_BASE as u64);
    for (pointer, id) in [(STATE_BASE, 7_u64), (HOST_BASE, 9)] {
        qword(&mut bytes, pointer, (IMAGE + VTABLE_RVA.0) as u64);
        qword(&mut bytes, pointer + HANDLE, (id << 32) | u64::from(TYPE));
    }
    qword(&mut bytes, STATE_BASE + STATE, STATE_OBJECT as u64);
    qword(&mut bytes, STATE_BASE + PROVINCE, PROVINCE_OBJECT as u64);
    dword(&mut bytes, STATE_BASE + CAPACITY, 1200);
    qword(
        &mut bytes,
        STATE_OBJECT,
        (IMAGE + cstate::VTABLE_RVA.0) as u64,
    );
    dword(&mut bytes, STATE_OBJECT + cstate::ID, 1);
    qword(
        &mut bytes,
        PROVINCE_OBJECT,
        (IMAGE + cprovince::VTABLE_RVA.0) as u64,
    );
    dword(&mut bytes, PROVINCE_OBJECT + cprovince::ID, 14);
    qword(
        &mut bytes,
        PROVINCE_OBJECT + cprovince::STATE,
        STATE_OBJECT as u64,
    );
    qword(&mut bytes, HOST_BASE + HOST_REFERENCE, (123_u64 << 32) | 51);
    dword(&mut bytes, HOST_BASE + CAPACITY, (-17_i32) as u32);
    dword(&mut bytes, HOST_BASE + KIND, 1);
    bytes
}

#[test]
fn state_holes_and_dense_hosts_preserve_native_fields_and_count() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: GAME,
        bytes: &bytes,
    };
    let mut output = vec![Row::default(); 4];
    assert_eq!(
        walk(&memory, IMAGE, GAME, &mut output),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        output[0],
        Row {
            id: (7_u64 << 32) | 65,
            state: Some(1),
            province: Some(14),
            host: 0,
            capacity: 1200,
            kind: 0,
        }
    );
    assert_eq!(
        output[1],
        Row {
            id: (9_u64 << 32) | 65,
            state: None,
            province: None,
            host: (123_u64 << 32) | 51,
            capacity: -17,
            kind: 1,
        }
    );
    assert_eq!(
        count(&memory, IMAGE, GAME),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2,
        }
    );
}

#[test]
fn malformed_membership_and_failed_rows_do_not_become_absence() {
    let mut bytes = fixture();
    let mut output = vec![Row::default(); 4];
    qword(&mut bytes, HOST_TABLE, 0);
    assert!(matches!(
        walk(
            &SliceMemory {
                base: GAME,
                bytes: &bytes
            },
            IMAGE,
            GAME,
            &mut output
        ),
        Outcome::Unavailable {
            reason: "stateless air base roster contains null base"
        }
    ));
    qword(&mut bytes, HOST_TABLE, HOST_BASE as u64);
    dword(&mut bytes, STATE_OBJECT + cstate::ID, 3);
    assert!(matches!(
        walk(
            &SliceMemory {
                base: GAME,
                bytes: &bytes
            },
            IMAGE,
            GAME,
            &mut output
        ),
        Outcome::Unavailable {
            reason: "air base state differs from roster slot"
        }
    ));
    dword(&mut bytes, STATE_OBJECT + cstate::ID, 1);
    qword(&mut bytes, PROVINCE_OBJECT + cprovince::STATE, 0);
    assert!(matches!(
        walk(
            &SliceMemory {
                base: GAME,
                bytes: &bytes
            },
            IMAGE,
            GAME,
            &mut output
        ),
        Outcome::Unavailable {
            reason: "air base province belongs to another state"
        }
    ));
}

const COUNTRY_TABLE: usize = 0x2800;
const LINK_TABLE: usize = 0x2840;
const COUNTRY_A: usize = 0x4100;
const COUNTRY_B: usize = 0x4200;
const CONTAINER_TABLE: usize = 0x2600;
const CONTAINER_A: usize = 0x3400;
const CONTAINER_B: usize = 0x3600;
const POOL_TABLE_A: usize = 0x2700;
const POOL_TABLE_B: usize = 0x2740;
const POOL_A: usize = 0x3800;
const POOL_B: usize = 0x3900;
const WING_TABLE_A: usize = 0x2780;
const WING_TABLE_B: usize = 0x27c0;
const WING_A: usize = 0x3a00;
const WING_B: usize = 0x3b00;

fn wing(bytes: &mut [u8], at: usize, handle: u64, pool: usize) {
    qword(bytes, at, (IMAGE + cair_wing::VTABLE_RVA.0) as u64);
    qword(
        bytes,
        at + cair_wing::SECONDARY,
        (IMAGE + cair_wing::SECONDARY_VTABLE_RVA.0) as u64,
    );
    qword(bytes, at + cair_wing::REFERENCE_TYPE, handle);
    qword(bytes, at + cair_wing::POOL, pool as u64);
}

fn registration_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRIES,
        COUNTRY_TABLE,
        3,
        3,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRY_LINKS,
        LINK_TABLE,
        10,
        10,
    );
    qword(&mut bytes, COUNTRY_TABLE + 8, COUNTRY_A as u64);
    qword(&mut bytes, COUNTRY_TABLE + 16, COUNTRY_B as u64);
    for (link, slot) in [(1_usize, 1_u32), (5, 1), (2, 2), (9, 2)] {
        dword(&mut bytes, LINK_TABLE + link * 4, slot);
    }
    for (pointer, id) in [(COUNTRY_A, 5_u32), (COUNTRY_B, 9)] {
        qword(&mut bytes, pointer, (IMAGE + ccountry::VTABLE_RVA.0) as u64);
        dword(&mut bytes, pointer + ccountry::ID, id);
    }
    array(
        &mut bytes,
        STATE_BASE + COUNTRY_CONTAINERS,
        CONTAINER_TABLE,
        4,
        4,
    );
    qword(&mut bytes, CONTAINER_TABLE, CONTAINER_A as u64);
    qword(&mut bytes, CONTAINER_TABLE + 16, CONTAINER_B as u64);
    for (pointer, country, pools, count) in [
        (CONTAINER_A, 0_u32, POOL_TABLE_A, 1_u32),
        (CONTAINER_B, 9, POOL_TABLE_B, 1),
    ] {
        qword(
            &mut bytes,
            pointer,
            (IMAGE + ccountry_air_container::VTABLE_RVA.0) as u64,
        );
        dword(
            &mut bytes,
            pointer + ccountry_air_container::COUNTRY,
            country,
        );
        array(
            &mut bytes,
            pointer + ccountry_air_container::POOLS,
            pools,
            count,
            count,
        );
    }
    qword(&mut bytes, POOL_TABLE_A, POOL_A as u64);
    qword(&mut bytes, POOL_TABLE_B, POOL_B as u64);
    for (pool, table, count) in [(POOL_A, WING_TABLE_A, 2_u32), (POOL_B, WING_TABLE_B, 0)] {
        qword(
            &mut bytes,
            pool,
            (IMAGE + cair_wing_pool::VTABLE_RVA.0) as u64,
        );
        array(
            &mut bytes,
            pool + cair_wing_pool::WINGS,
            table,
            count.max(1),
            count,
        );
    }
    qword(&mut bytes, WING_TABLE_A, WING_A as u64);
    qword(&mut bytes, WING_TABLE_A + 8, WING_B as u64);
    wing(
        &mut bytes,
        WING_A,
        (11_u64 << 32) | u64::from(cair_wing::TYPE),
        POOL_A,
    );
    wing(&mut bytes, WING_B, u64::MAX, POOL_A);
    bytes
}

fn registered(
    bytes: &[u8],
    rows: usize,
    arena: usize,
) -> (RegistrationWalk, Vec<RegistrationRow>, Vec<u64>) {
    let memory = SliceMemory { base: GAME, bytes };
    let [first, rest @ ..] = [0, 1, 3, 10, 16].map(|cache| {
        let mut registrations = vec![RegistrationRow::default(); rows];
        let mut wings = vec![0; arena];
        let mut resolved = vec![Some(stale_country()); cache];
        let walk = walk_registrations(
            &memory,
            IMAGE,
            GAME,
            RegistrationBuffers {
                registrations: &mut registrations,
                wings: &mut wings,
                resolved: &mut resolved,
            },
        );
        (walk, registrations, wings)
    });
    for other in rest {
        assert_eq!(other, first);
    }
    first
}

fn stale_country() -> ccountry::Country {
    ccountry::Country {
        pointer: 0xdead,
        canonical_slot: 1,
        id: 9,
    }
}

#[test]
fn registered_slots_keep_native_order_reserved_slots_and_empty_containers() {
    let (walk, rows, wings) = registered(&registration_fixture(), 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    assert_eq!((walk.wings_len, walk.required_wings), (2, 2));
    assert_eq!(
        rows[0],
        RegistrationRow {
            base: (7_u64 << 32) | 65,
            slot: 0,
            capacity_raw: 0,
            wings_start: 0,
            wings_len: 2,
        }
    );
    assert_eq!(
        rows[1],
        RegistrationRow {
            base: (7_u64 << 32) | 65,
            slot: 2,
            capacity_raw: 0,
            wings_start: 2,
            wings_len: 0,
        }
    );
    assert_eq!(
        &wings[..2],
        [(11_u64 << 32) | u64::from(cair_wing::TYPE), u64::MAX]
    );
}

#[test]
fn registration_scratch_reuse_preserves_shorter_bases_and_fault_prefixes() {
    use std::cell::RefCell;

    const SHORT_CONTAINERS: usize = 0x2a00;
    const SHORT_POOLS: usize = 0x2a10;
    const SHORT_WINGS: usize = 0x2a20;
    const CONTAINER: usize = 0x3c00;
    const POOL: usize = 0x3d00;
    const WING: usize = 0x3e00;
    let mut bytes = registration_fixture();
    array(
        &mut bytes,
        CONTAINER_A + ccountry_air_container::POOLS,
        POOL_TABLE_A,
        2,
        2,
    );
    qword(&mut bytes, POOL_TABLE_A + 8, POOL_B as u64);
    array(
        &mut bytes,
        POOL_A + cair_wing_pool::WINGS,
        WING_TABLE_A,
        3,
        3,
    );
    qword(&mut bytes, WING_TABLE_A + 8, WING_A as u64);
    qword(&mut bytes, WING_TABLE_A + 16, WING_B as u64);
    for (header, data) in [
        (HOST_BASE + COUNTRY_CONTAINERS, SHORT_CONTAINERS),
        (CONTAINER + ccountry_air_container::POOLS, SHORT_POOLS),
        (POOL + cair_wing_pool::WINGS, SHORT_WINGS),
    ] {
        array(&mut bytes, header, data, 1, 1);
    }
    for (at, value) in [
        (SHORT_CONTAINERS, CONTAINER),
        (SHORT_POOLS, POOL),
        (SHORT_WINGS, WING),
        (CONTAINER, IMAGE + ccountry_air_container::VTABLE_RVA.0),
        (POOL, IMAGE + cair_wing_pool::VTABLE_RVA.0),
    ] {
        qword(&mut bytes, at, value as u64);
    }
    wing(&mut bytes, WING, u64::MAX, POOL);
    let (walk, rows, wings) = registered(&bytes, 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!((walk.wings_len, walk.required_wings), (4, 4));
    let repeated = (11_u64 << 32) | u64::from(cair_wing::TYPE);
    assert_eq!(&wings[..4], [repeated, repeated, u64::MAX, u64::MAX]);
    assert_eq!(
        rows[2],
        RegistrationRow {
            base: (9_u64 << 32) | u64::from(TYPE),
            slot: 0,
            capacity_raw: 0,
            wings_start: 3,
            wings_len: 1,
        }
    );

    struct Traced<'a> {
        bytes: &'a [u8],
        blocked: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self
                .blocked
                .is_some_and(|blocked| (address..address + destination.len()).contains(&blocked))
            {
                destination[0] = 0xff;
                return false;
            }
            SliceMemory {
                base: GAME,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    for blocked in [
        None,
        Some(HOST_BASE + COUNTRY_CONTAINERS),
        Some(SHORT_CONTAINERS),
        Some(CONTAINER),
        Some(CONTAINER + ccountry_air_container::COUNTRY),
        Some(CONTAINER + ccountry_air_container::POOLS),
        Some(SHORT_POOLS),
        Some(POOL),
        Some(POOL + cair_wing_pool::WINGS),
        Some(SHORT_WINGS),
        Some(WING + cair_wing::POOL + 7),
    ] {
        let run = |reset_each_base| {
            let memory = Traced {
                bytes: &bytes,
                blocked,
                reads: RefCell::new(Vec::new()),
            };
            let reader = Reader {
                memory: &memory,
                base: IMAGE,
            };
            let countries = ccountry::Reader::new(&memory, IMAGE, GAME).unwrap();
            let mut cells = [None; 16];
            let mut resolved = Resolver {
                cells: &mut cells,
            };
            let mut state = RegistrationState::default();
            state.scratch.container_bytes.fill(0xa5);
            state.scratch.pool_pointers.fill(usize::MAX);
            state.scratch.pool_records.fill(0xa5);
            state.scratch.pool_pointer_bytes.fill(0xa5);
            state.scratch.wing_pointers.fill(usize::MAX);
            state.scratch.wing_records.fill(0xa5);
            state.scratch.wing_pointer_bytes.fill(0xa5);
            let mut rows = [RegistrationRow::default(); 4];
            let mut wings = [0; 8];
            let mut buffers = RegistrationBuffers {
                registrations: &mut rows,
                wings: &mut wings,
                resolved: &mut [],
            };
            let result = [(STATE_BASE, 7_u64), (HOST_BASE, 9)]
                .into_iter()
                .try_for_each(|(pointer, id)| {
                    if reset_each_base {
                        state.scratch = RegistrationScratch::default();
                    }
                    reader.registration::<Traced<'_>>(
                        RegistrationBase {
                            pointer,
                            handle: (id << 32) | u64::from(TYPE),
                        },
                        &countries,
                        &mut resolved,
                        &mut buffers,
                        &mut state,
                        &mut None,
                        super::SlotMode::default(),
                        None,
                    )
                });
            (
                result,
                (
                    state.len,
                    state.unread_rows,
                    state.wings_len,
                    state.required_wings,
                ),
                rows,
                wings,
                memory.reads.into_inner(),
            )
        };
        let reused = run(false);
        assert_eq!(reused, run(true), "blocked={blocked:x?}");
        assert_eq!(reused.0.is_ok(), blocked.is_none());
    }
}

#[test]
fn registration_capacity_releases_staged_wings_and_reports_full_demand() {
    let (walk, rows, wings) = registered(&registration_fixture(), 4, 1);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!((walk.wings_len, walk.required_wings), (0, 2));
    assert_eq!(
        (rows[0].slot, rows[0].wings_start, rows[0].wings_len),
        (2, 0, 0)
    );
    assert!(wings[..walk.wings_len].is_empty());
    let (walk, _, _) = registered(&registration_fixture(), 0, 8);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!(walk.required_wings, 2);
}

#[test]
fn registration_wings_admit_only_two_complete_fields_and_keep_the_fault_prefix() {
    use std::cell::RefCell;

    struct Observed<'a> {
        bytes: &'a [u8],
        fault: usize,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if (address..address + output.len()).contains(&self.fault) {
                output.fill(0xa5);
                return false;
            }
            SliceMemory {
                base: GAME,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }

    let bytes = registration_fixture();
    let fields = [
        (WING_A, 32),
        (WING_A + cair_wing::POOL, 8),
        (WING_B, 32),
        (WING_B + cair_wing::POOL, 8),
    ];
    for (field_index, &(address, size)) in fields.iter().enumerate() {
        for byte in 0..size {
            let memory = Observed {
                bytes: &bytes,
                fault: address + byte,
                reads: Default::default(),
            };
            let mut rows = [RegistrationRow::default(); 4];
            let mut wings = [u64::MAX; 8];
            let mut resolved = [None; 16];
            let walked = walk_registrations(
                &memory,
                IMAGE,
                GAME,
                RegistrationBuffers {
                    registrations: &mut rows,
                    wings: &mut wings,
                    resolved: &mut resolved,
                },
            );
            assert_eq!(
                walked.outcome,
                Outcome::Unavailable {
                    reason: "air pool wing identity unreadable"
                }
            );
            assert_eq!(walked.wings_len, field_index / 2);
            assert_eq!(walked.required_wings, 2);
            assert_eq!(rows, [RegistrationRow::default(); 4]);
            let observed: Vec<_> = memory
                .reads
                .borrow()
                .iter()
                .copied()
                .filter(|read| fields.contains(read))
                .collect();
            assert_eq!(
                observed,
                fields[..=field_index],
                "fault {:#x}",
                address + byte
            );
            if walked.wings_len != 0 {
                assert_eq!(wings[0], (11_u64 << 32) | u64::from(cair_wing::TYPE));
            }
        }
    }
}

#[test]
fn registration_faults_never_become_absence() {
    for (address, value, reason) in [
        (
            CONTAINER_A,
            0x1234_u64,
            "air base or linked object vtable mismatch",
        ),
        (
            POOL_TABLE_A,
            0,
            "air container pool pointer unreadable or null",
        ),
        (WING_TABLE_A, 0, "air pool wing pointer unreadable or null"),
        (
            WING_A + cair_wing::REFERENCE_TYPE,
            0,
            "registered air wing has a null reference handle",
        ),
        (
            WING_A + cair_wing::SECONDARY,
            0x1234,
            "air pool wing vtable mismatch",
        ),
        (
            WING_A + cair_wing::POOL,
            POOL_B as u64,
            "registered air wing backlink differs from containing pool",
        ),
        (
            WING_B + cair_wing::POOL,
            0,
            "registered air wing backlink differs from containing pool",
        ),
        (
            CONTAINER_B + ccountry_air_container::COUNTRY,
            5,
            "air base container country differs from its slot",
        ),
        (
            CONTAINER_A + ccountry_air_container::COUNTRY,
            5,
            "air base container at the reserved slot is linked to a country slot",
        ),
    ] {
        let mut bytes = registration_fixture();
        qword(&mut bytes, address, value);
        assert_eq!(
            registered(&bytes, 4, 8).0.outcome,
            Outcome::Unavailable { reason },
            "{address:#x}"
        );
    }
    let mut bytes = registration_fixture();
    dword(&mut bytes, POOL_A + cair_wing_pool::WINGS_COUNT, 9);
    assert_eq!(
        registered(&bytes, 4, 8).0.outcome,
        Outcome::Unavailable {
            reason: "air pool wing vector unreadable or invalid"
        }
    );
}

#[test]
fn a_reserved_slot_accepts_an_unlinked_reference_and_keeps_its_capacity() {
    let mut bytes = registration_fixture();
    dword(&mut bytes, CONTAINER_A + ccountry_air_container::COUNTRY, 7);
    qword(
        &mut bytes,
        CONTAINER_A + ccountry_air_container::CAPACITY_RAW,
        ccountry_air_container::INITIAL_CAPACITY_RAW as u64,
    );
    let (walk, rows, _) = registered(&bytes, 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        (rows[0].slot, rows[0].capacity_raw),
        (0, ccountry_air_container::INITIAL_CAPACITY_RAW)
    );
    dword(&mut bytes, LINK_TABLE + 7 * 4, 4);
    assert_eq!(
        registered(&bytes, 4, 8).0.outcome,
        Outcome::Unavailable {
            reason: "country link unreadable or outside country table"
        }
    );
}

#[test]
fn a_base_without_containers_is_an_honest_empty_registration_table() {
    let mut bytes = registration_fixture();
    dword(&mut bytes, STATE_BASE + COUNTRY_CONTAINERS_COUNT, 0);
    let (walk, _, _) = registered(&bytes, 4, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
    assert_eq!((walk.wings_len, walk.required_wings), (0, 0));
}

struct Rounds<'a> {
    inner: SliceMemory<'a>,
    rounds: std::cell::RefCell<Vec<usize>>,
}

impl Memory for Rounds<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.rounds.borrow_mut().push(address);
        self.inner.copy(address, destination)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.rounds
            .borrow_mut()
            .push(sources.pointer(0).unwrap_or(0));
        struct Plain<'a, 'b>(&'a SliceMemory<'b>);
        impl Memory for Plain<'_, '_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.0.copy(address, destination)
            }
        }
        Plain(&self.inner).copy_records(sources, layout, destination)
    }
}

#[test]
fn country_registrations_match_with_any_resolver_length() {
    const COUNTRIES: u32 = 129;
    const LINKS: u32 = COUNTRIES * 2;
    const HOSTS: usize = 0x2100;
    const BASE_A: usize = 0x2200;
    const BASE_B: usize = 0x2300;
    const TABLE: usize = 0x3000;
    const LINK: usize = 0x10000;
    const OBJECTS: usize = 0x20000;
    const SLOTS_A: usize = 0x40000;
    const SLOTS_B: usize = 0x50000;
    const CONTAINERS: usize = 0x60000;
    let mut bytes = vec![0; CONTAINERS + COUNTRIES as usize * 0x100];
    qword(
        &mut bytes,
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    qword(
        &mut bytes,
        MANAGER,
        (IMAGE + cstrategic_air_manager::VTABLE_RVA.0) as u64,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOSTS,
        2,
        2,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRIES,
        TABLE,
        COUNTRIES,
        COUNTRIES,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRY_LINKS,
        LINK,
        LINKS,
        LINKS,
    );
    for (index, (base, slots, id)) in [(BASE_A, SLOTS_A, 7_u64), (BASE_B, SLOTS_B, 9)]
        .into_iter()
        .enumerate()
    {
        qword(&mut bytes, HOSTS + index * 8, base as u64);
        qword(&mut bytes, base, (IMAGE + VTABLE_RVA.0) as u64);
        qword(&mut bytes, base + HANDLE, (id << 32) | u64::from(TYPE));
        array(
            &mut bytes,
            base + COUNTRY_CONTAINERS,
            slots,
            COUNTRIES,
            COUNTRIES,
        );
    }
    let mut expected = Vec::new();
    for slot in 1..COUNTRIES {
        let object = OBJECTS + slot as usize * 16;
        qword(&mut bytes, TABLE + slot as usize * 8, object as u64);
        qword(&mut bytes, object, (IMAGE + ccountry::VTABLE_RVA.0) as u64);
        dword(&mut bytes, object + ccountry::ID, slot);
        dword(&mut bytes, LINK + slot as usize * 4, slot);
        dword(&mut bytes, LINK + (slot + COUNTRIES) as usize * 4, slot);
        let raw = if slot % 2 == 1 {
            slot + COUNTRIES
        } else {
            slot
        };
        let container = CONTAINERS + slot as usize * 0x100;
        qword(
            &mut bytes,
            container,
            (IMAGE + ccountry_air_container::VTABLE_RVA.0) as u64,
        );
        dword(&mut bytes, container + ccountry_air_container::COUNTRY, raw);
        qword(
            &mut bytes,
            container + ccountry_air_container::CAPACITY_RAW,
            u64::from(raw),
        );
        qword(&mut bytes, SLOTS_A + slot as usize * 8, container as u64);
        expected.push(((7_u64 << 32) | u64::from(TYPE), slot, i64::from(raw)));
        if slot % 7 == 0 {
            qword(&mut bytes, SLOTS_B + slot as usize * 8, container as u64);
        }
    }
    expected.extend((1..COUNTRIES).filter(|slot| slot % 7 == 0).map(|slot| {
        let raw = if slot % 2 == 1 {
            slot + COUNTRIES
        } else {
            slot
        };
        ((9_u64 << 32) | u64::from(TYPE), slot, i64::from(raw))
    }));
    let run = |cache: usize| {
        let memory = Rounds {
            inner: SliceMemory {
                base: GAME,
                bytes: &bytes,
            },
            rounds: Default::default(),
        };
        let mut registrations = vec![RegistrationRow::default(); expected.len()];
        let mut wings = [0; 1];
        let mut resolved = vec![None; cache];
        let walk = walk_registrations(
            &memory,
            IMAGE,
            GAME,
            RegistrationBuffers {
                registrations: &mut registrations,
                wings: &mut wings,
                resolved: &mut resolved,
            },
        );
        (walk, registrations, memory.rounds.into_inner())
    };
    let (walk, rows, rounds) = run(LINKS as usize);
    assert_eq!(
        walk.outcome,
        Outcome::Rows {
            len: expected.len()
        }
    );
    assert_eq!(walk.required_links, LINKS as usize);
    assert_eq!(
        rows.iter()
            .map(|row| (row.base, row.slot, row.capacity_raw))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(rows.iter().all(|row| row.wings_len == 0));
    let chunk_rounds = rounds
        .iter()
        .filter(|&&at| {
            (SLOTS_A..SLOTS_A + COUNTRIES as usize * 8).contains(&at)
                || (SLOTS_B..SLOTS_B + COUNTRIES as usize * 8).contains(&at)
        })
        .count();
    assert_eq!(chunk_rounds, 2 * (COUNTRIES as usize).div_ceil(BATCH));
    for cache in [0, 64, COUNTRIES as usize] {
        let (short, short_rows, short_rounds) = run(cache);
        assert_eq!((short, &short_rows), (walk, &rows), "cache {cache}");
        assert!(short_rounds.len() > rounds.len(), "cache {cache}");
    }
    let resolves = |rounds: &[usize]| {
        rounds
            .iter()
            .filter(|&&at| (LINK..LINK + LINKS as usize * 4).contains(&at))
            .count()
    };
    let (_, _, uncached) = run(0);
    assert!(resolves(&rounds) < resolves(&uncached));
}

const SPARSE_COUNTRIES: u32 = 129;
const SPARSE_HOSTS: usize = 0x2100;
const SPARSE_BASE: usize = 0x2200;
const SPARSE_DENSE: usize = 0x2400;
const SPARSE_TABLE: usize = 0x3000;
const SPARSE_LINK: usize = 0x10000;
const SPARSE_OBJECTS: usize = 0x20000;
const SPARSE_SLOTS: usize = 0x40000;
const SPARSE_CONTAINERS: usize = 0x60000;
const SPARSE_OCCUPIED: [u32; 3] = [1, 70, 128];

fn sparse_container(index: usize) -> usize {
    SPARSE_CONTAINERS + index * 0x100
}

fn sparse_slot_fixture(listed: &[usize]) -> Vec<u8> {
    let mut bytes = vec![0; sparse_container(SPARSE_OCCUPIED.len()) - GAME];
    qword(
        &mut bytes,
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    qword(
        &mut bytes,
        MANAGER,
        (IMAGE + cstrategic_air_manager::VTABLE_RVA.0) as u64,
    );
    array(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        SPARSE_HOSTS,
        1,
        1,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRIES,
        SPARSE_TABLE,
        SPARSE_COUNTRIES,
        SPARSE_COUNTRIES,
    );
    array(
        &mut bytes,
        GAME + cgame_state::COUNTRY_LINKS,
        SPARSE_LINK,
        SPARSE_COUNTRIES,
        SPARSE_COUNTRIES,
    );
    qword(&mut bytes, SPARSE_HOSTS, SPARSE_BASE as u64);
    qword(&mut bytes, SPARSE_BASE, (IMAGE + VTABLE_RVA.0) as u64);
    qword(
        &mut bytes,
        SPARSE_BASE + HANDLE,
        (7_u64 << 32) | u64::from(TYPE),
    );
    array(
        &mut bytes,
        SPARSE_BASE + COUNTRY_CONTAINERS,
        SPARSE_SLOTS,
        SPARSE_COUNTRIES,
        SPARSE_COUNTRIES,
    );
    let count = listed.len() as u32;
    array(
        &mut bytes,
        SPARSE_BASE + REGISTERED_CONTAINERS,
        SPARSE_DENSE,
        count,
        count,
    );
    for (entry, &index) in listed.iter().enumerate() {
        qword(
            &mut bytes,
            SPARSE_DENSE + entry * 8,
            sparse_container(index) as u64,
        );
    }
    for (index, slot) in SPARSE_OCCUPIED.into_iter().enumerate() {
        let object = SPARSE_OBJECTS + slot as usize * 16;
        qword(&mut bytes, SPARSE_TABLE + slot as usize * 8, object as u64);
        qword(&mut bytes, object, (IMAGE + ccountry::VTABLE_RVA.0) as u64);
        dword(&mut bytes, object + ccountry::ID, slot);
        dword(&mut bytes, SPARSE_LINK + slot as usize * 4, slot);
        let container = sparse_container(index);
        qword(
            &mut bytes,
            container,
            (IMAGE + ccountry_air_container::VTABLE_RVA.0) as u64,
        );
        dword(
            &mut bytes,
            container + ccountry_air_container::COUNTRY,
            slot,
        );
        qword(
            &mut bytes,
            container + ccountry_air_container::CAPACITY_RAW,
            u64::from(slot),
        );
        qword(
            &mut bytes,
            SPARSE_SLOTS + slot as usize * 8,
            container as u64,
        );
    }
    bytes
}

fn sparse_walk(bytes: &[u8], dense: bool) -> (RegistrationWalk, Vec<RegistrationRow>, Vec<usize>) {
    let memory = Rounds {
        inner: SliceMemory { base: GAME, bytes },
        rounds: Default::default(),
    };
    let mut registrations = vec![RegistrationRow::default(); 4];
    let mut wings = [0; 1];
    let mut resolved = vec![None; 16];
    let walk = walk_registrations_planned(
        &memory,
        IMAGE,
        GAME,
        RegistrationBuffers {
            registrations: &mut registrations,
            wings: &mut wings,
            resolved: &mut resolved,
        },
        SlotMode { dense },
        None,
    );
    (walk, registrations, memory.rounds.into_inner())
}

fn slot_vector_rounds(rounds: &[usize]) -> Vec<usize> {
    let mut rounds: Vec<usize> = rounds
        .iter()
        .copied()
        .filter(|at| (SPARSE_SLOTS..SPARSE_SLOTS + SPARSE_COUNTRIES as usize * 8).contains(at))
        .collect();
    rounds.sort_unstable();
    rounds
}

#[test]
fn a_sparse_base_reads_only_the_slot_words_of_its_listed_containers() {
    let bytes = sparse_slot_fixture(&[2, 0, 1]);
    let (full, full_rows, full_rounds) = sparse_walk(&bytes, false);
    assert_eq!(full.outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        full_rows[..3]
            .iter()
            .map(|row| (row.slot, row.capacity_raw))
            .collect::<Vec<_>>(),
        [(1, 1), (70, 70), (128, 128)]
    );
    assert_eq!(full.dense_bases, 0);
    assert_eq!(
        slot_vector_rounds(&full_rounds).len(),
        (SPARSE_COUNTRIES as usize).div_ceil(BATCH)
    );

    let (dense, dense_rows, dense_rounds) = sparse_walk(&bytes, true);
    assert_eq!(dense.dense_bases, 1);
    assert_eq!(
        (dense.outcome, dense.wings_len, dense.required_wings),
        (full.outcome, full.wings_len, full.required_wings)
    );
    assert_eq!(dense_rows, full_rows);
    let expected: Vec<usize> = SPARSE_OCCUPIED
        .iter()
        .map(|&slot| SPARSE_SLOTS + slot as usize * 8)
        .collect();
    assert_eq!(slot_vector_rounds(&dense_rounds), expected);
}

#[test]
fn a_stale_or_disagreeing_dense_list_falls_back_with_an_unchanged_outcome() {
    let check = |bytes: &[u8], what: &str| {
        let (full, full_rows, _) = sparse_walk(bytes, false);
        let (dense, dense_rows, _) = sparse_walk(bytes, true);
        assert_eq!(dense.dense_bases, 0, "{what}");
        assert_eq!(
            (dense.outcome, &dense_rows),
            (full.outcome, &full_rows),
            "{what}"
        );
        full.outcome
    };
    let bytes = sparse_slot_fixture(&[0, 0, 2]);
    assert_eq!(check(&bytes, "listed twice"), Outcome::Rows { len: 3 });
    let mut bytes = sparse_slot_fixture(&[0, 1, 2]);
    array(
        &mut bytes,
        SPARSE_BASE + REGISTERED_CONTAINERS,
        SPARSE_DENSE,
        SPARSE_COUNTRIES + 1,
        SPARSE_COUNTRIES + 1,
    );
    assert_eq!(check(&bytes, "longer than slots"), Outcome::Rows { len: 3 });
    let mut bytes = sparse_slot_fixture(&[0, 1, 2]);
    qword(&mut bytes, SPARSE_SLOTS + 70 * 8, 0);
    qword(
        &mut bytes,
        SPARSE_SLOTS + 71 * 8,
        sparse_container(1) as u64,
    );
    assert_eq!(
        check(&bytes, "stale entry"),
        Outcome::Unavailable {
            reason: "air base container country differs from its slot"
        }
    );
    let mut bytes = sparse_slot_fixture(&[0, 1, 2]);
    array(
        &mut bytes,
        SPARSE_BASE + COUNTRY_CONTAINERS,
        SPARSE_SLOTS,
        100,
        100,
    );

    assert_eq!(check(&bytes, "slot out of range"), Outcome::Rows { len: 2 });
    let bytes = sparse_slot_fixture(&[]);
    let (dense, _, _) = sparse_walk(&bytes, true);
    assert_eq!(
        (dense.dense_bases, dense.outcome),
        (1, Outcome::Rows { len: 0 })
    );
}

#[test]
fn growing_air_bases_follow_mod_stateless_rosters_before_first_publication() {
    let mut bytes = fixture();
    let prototype = bytes[HOST_BASE - GAME..HOST_BASE - GAME + SIZE].to_vec();
    let data = 0x10000;
    let objects = 0x40000;
    let mut rows = Vec::new();
    for count in [50_u32, 120] {
        bytes.resize(objects - GAME + count as usize * SIZE, 0);
        array(
            &mut bytes,
            MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
            data,
            count,
            count,
        );
        for index in 0..count as usize {
            let pointer = objects + index * SIZE;
            bytes[pointer - GAME..pointer - GAME + SIZE].copy_from_slice(&prototype);
            qword(
                &mut bytes,
                pointer + HANDLE,
                ((index as u64 + 10) << 32) | u64::from(TYPE),
            );
            qword(&mut bytes, data + index * 8, pointer as u64);
        }
        let memory = SliceMemory {
            base: GAME,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, IMAGE, GAME, &mut rows),
            Outcome::Rows {
                len: count as usize + 1
            }
        );
        assert_eq!(
            rows[count as usize].id,
            ((count as u64 + 9) << 32) | u64::from(TYPE)
        );
    }
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
#[ignore = "run alone with alloc-fault-injection: allocation hooks are process-global"]
fn carrier_advisory_denial_keeps_capture_active_and_original_rows_readable() {
    use engine_allocation as allocation;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    static DENY: AtomicBool = AtomicBool::new(false);
    static FAILURES: AtomicUsize = AtomicUsize::new(0);
    fn before() -> bool {
        DENY.load(Ordering::SeqCst)
    }
    fn failed() {
        FAILURES.fetch_add(1, Ordering::SeqCst);
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            allocation::set_hooks(None, None);
        }
    }
    let _reset = Reset;
    let bytes = host_registration_fixture();
    let memory = CarrierMemory {
        bytes: &bytes,
        fault: None,
        globals: carrier_globals(),
        reads: std::cell::RefCell::new(Vec::with_capacity(4_096)),
    };
    allocation::set_hooks(Some(before), Some(failed));
    DENY.store(true, Ordering::SeqCst);
    let hint =
        allocation::test_support::with_allocations_denied(|| carrier_demand(&memory, IMAGE, GAME))
            .unwrap();
    assert_eq!(hint.failure, Some("carrier raw sizing allocation failed"));
    assert_eq!(FAILURES.load(Ordering::SeqCst), 0);
    assert!(!allocation::capture_stopped());
    DENY.store(false, Ordering::SeqCst);
    let rows = carrier_walk_fixture(&bytes, None, 4, 4);
    assert_eq!(rows.main.outcome, Outcome::Rows { len: 2 });
    assert_eq!(FAILURES.load(Ordering::SeqCst), 0);
}

struct KeptBase {
    handle: u64,
    pointer: usize,
    slots: u32,
    sought_plan: bool,
    plan: Option<Vec<DenseEntry>>,
    rows: Vec<RegistrationRow>,
    wings: Vec<u64>,
    countries: Vec<u32>,
    pools: usize,
    required_wings: usize,
}

#[derive(Default)]
struct Keep {
    bases: Vec<KeptBase>,
    walking: Option<(u64, usize)>,
    countries: Vec<u32>,
    reused: Vec<u64>,
}

impl RegistrationReuse for Keep {
    fn previous(&self, handle: u64, pointer: usize) -> Option<HeldBase<'_>> {
        let kept = self
            .bases
            .iter()
            .find(|kept| kept.handle == handle && kept.pointer == pointer)?;
        Some(HeldBase {
            output: WalkedBase {
                slots: kept.slots,
                sought_plan: kept.sought_plan,
                plan: kept.plan.as_deref(),
                rows: &kept.rows,
                wings_start: 0,
                wings: &kept.wings,
                pools: kept.pools,
                required_wings: kept.required_wings,
            },
            countries: &kept.countries,
        })
    }

    fn reused(&mut self, handle: u64, _pointer: usize) {
        self.reused.push(handle);
    }

    fn begin(&mut self, handle: u64, pointer: usize) {
        self.walking = Some((handle, pointer));
        self.countries.clear();
    }

    fn container(&mut self, country: u32) {
        self.countries.push(country);
    }

    fn walked(&mut self, output: WalkedBase<'_>) {
        let (handle, pointer) = self.walking.take().unwrap();
        self.bases.retain(|kept| kept.handle != handle);
        self.bases.push(KeptBase {
            handle,
            pointer,
            slots: output.slots,
            sought_plan: output.sought_plan,
            plan: output.plan.map(<[DenseEntry]>::to_vec),
            rows: output
                .rows
                .iter()
                .map(|row| RegistrationRow {
                    wings_start: row.wings_start - output.wings_start,
                    ..*row
                })
                .collect(),
            wings: output.wings.to_vec(),
            countries: std::mem::take(&mut self.countries),
            pools: output.pools,
            required_wings: output.required_wings,
        });
    }
}

fn kept_walk(
    bytes: &[u8],
    keep: &mut Keep,
    rows: usize,
    arena: usize,
    dense: bool,
) -> (RegistrationWalk, Vec<RegistrationRow>, Vec<u64>, Vec<usize>) {
    let memory = Rounds {
        inner: SliceMemory { base: GAME, bytes },
        rounds: Default::default(),
    };
    let mut registrations = vec![RegistrationRow::default(); rows];
    let mut wings = vec![0; arena];
    let mut resolved = vec![None; 16];
    let walk = walk_registrations_planned(
        &memory,
        IMAGE,
        GAME,
        RegistrationBuffers {
            registrations: &mut registrations,
            wings: &mut wings,
            resolved: &mut resolved,
        },
        SlotMode { dense },
        Some(keep),
    );
    (walk, registrations, wings, memory.rounds.into_inner())
}

fn without_reuse(walk: RegistrationWalk) -> RegistrationWalk {
    RegistrationWalk {
        reused_bases: 0,
        ..walk
    }
}

#[test]
fn a_kept_state_base_appends_its_output_without_reading_its_subtree() {
    let bytes = registration_fixture();
    let (plain, plain_rows, plain_wings) = registered(&bytes, 4, 8);
    let mut keep = Keep::default();
    let (first, rows, wings, _) = kept_walk(&bytes, &mut keep, 4, 8, false);
    assert_eq!((first, &rows, &wings), (plain, &plain_rows, &plain_wings));
    assert_eq!(first.reused_bases, 0);
    assert_eq!(
        keep.bases
            .iter()
            .map(|kept| kept.handle)
            .collect::<Vec<_>>(),
        [(7_u64 << 32) | u64::from(TYPE)]
    );
    assert_eq!(keep.bases[0].countries, [0, 9]);

    let (second, rows, wings, rounds) = kept_walk(&bytes, &mut keep, 4, 8, false);
    assert_eq!(second.reused_bases, 1);
    assert_eq!(
        (without_reuse(second), &rows, &wings),
        (plain, &plain_rows, &plain_wings)
    );
    for object in [
        CONTAINER_A,
        CONTAINER_B,
        POOL_A,
        POOL_B,
        WING_A,
        WING_B,
        POOL_TABLE_A,
        WING_TABLE_A,
    ] {
        assert!(
            !rounds
                .iter()
                .any(|&at| (object..object + 0x40).contains(&at)),
            "read {object:#x}"
        );
    }
}

#[test]
fn a_kept_state_base_is_walked_when_its_countries_plan_or_arenas_differ() {
    let bytes = registration_fixture();
    let mut keep = Keep::default();
    kept_walk(&bytes, &mut keep, 4, 8, false);

    let mut moved = bytes.clone();
    dword(&mut moved, LINK_TABLE + 9 * 4, 1);
    let (plain, plain_rows, _) = registered(&moved, 4, 8);
    assert!(matches!(plain.outcome, Outcome::Unavailable { .. }));
    let mut offered = Keep::default();
    kept_walk(&bytes, &mut offered, 4, 8, false);
    let (walk, rows, _, _) = kept_walk(&moved, &mut offered, 4, 8, false);
    assert_eq!(walk.reused_bases, 0);
    assert_eq!((walk, &rows), (plain, &plain_rows));

    let mut offered = Keep::default();
    kept_walk(&bytes, &mut offered, 4, 8, false);
    let (walk, _, _, _) = kept_walk(&bytes, &mut offered, 4, 8, true);
    assert_eq!(walk.reused_bases, 0);

    let (plain, plain_rows, plain_wings) = registered(&bytes, 1, 8);
    let mut offered = Keep::default();
    kept_walk(&bytes, &mut offered, 4, 8, false);
    let (walk, rows, wings, _) = kept_walk(&bytes, &mut offered, 1, 8, false);
    assert_eq!(walk.reused_bases, 0);
    assert_eq!((walk, &rows, &wings), (plain, &plain_rows, &plain_wings));
}

#[test]
fn state_summary_country_order_preserves_repeats_zero_slots_and_zero_capacity() {
    const DENSE: usize = 0x2b00;
    let mut bytes = registration_fixture();
    array(&mut bytes, STATE_BASE + REGISTERED_CONTAINERS, DENSE, 3, 3);
    for (index, pointer) in [CONTAINER_B, CONTAINER_A, CONTAINER_B]
        .into_iter()
        .enumerate()
    {
        qword(&mut bytes, DENSE + index * 8, pointer as u64);
    }
    let run = |bytes: &[u8], handle| {
        let memory = SliceMemory { base: GAME, bytes };
        let countries = ccountry::Reader::new(&memory, IMAGE, GAME).unwrap();
        let mut output = [CountryOccurrence::default(); 3];
        let result = state_country_order(
            &memory,
            IMAGE,
            STATE_BASE,
            &Row {
                id: handle,
                state: Some(1),
                ..Default::default()
            },
            &countries,
            &mut [None; 10],
            &mut output,
            &mut [0; 24],
        );
        (result, output)
    };
    let handle = (7_u64 << 32) | u64::from(TYPE);
    let (result, output) = run(&bytes, handle);
    assert_eq!(result, Ok(3));
    assert_eq!(
        output,
        [
            CountryOccurrence {
                country: 9,
                slot: 2
            },
            CountryOccurrence {
                country: 0,
                slot: 0
            },
            CountryOccurrence {
                country: 9,
                slot: 2
            }
        ]
    );
    assert!(run(&bytes, handle + (1_u64 << 32)).0.is_err());
    qword(&mut bytes, CONTAINER_TABLE + 16, 0);
    assert_eq!(
        run(&bytes, handle).0,
        Err("state air-base dense country differs from its indexed container")
    );
}
