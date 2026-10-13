use super::*;
use engine::cair_base::{CarrierPoolFuel, ContainerFuel};

fn lengths() -> CarrierLengths {
    CarrierLengths {
        bases: 1,
        registrations: 1,
        pools: 3,
        countries: 3,
        names: 0,
    }
}

#[test]
fn fresh_job_presizing_initializes_fuel_scratch_before_its_first_callback() {
    let mut job = Job::new();
    let demand = CarrierDemand {
        bases: 5,
        registrations: 7,
        pools: 11,
        indexed: 5_000,
        resolved: 5_000,
        country_terms: 5_000,
        aces: 26,
        forecasts: 26,
        ..Default::default()
    };
    job.presize(demand);
    assert!(job.registrations.pools.len() >= 11);
    for (actual, required) in job
        .scratch
        .capacity()
        .into_iter()
        .zip([5_000, 5_000, 5_000, 26, 26])
    {
        assert!(actual >= required);
    }
    assert_eq!(job.demand, CarrierDemand::default());
    assert!(job.log.borrow().spans.is_empty());
    job.clear();
    for (actual, required) in job
        .scratch
        .capacity()
        .into_iter()
        .zip([5_000, 5_000, 5_000, 26, 26])
    {
        assert!(actual >= required);
    }
}

fn outputs() -> (Bases, Registrations) {
    let mut bases = Bases::new(1);
    bases.prepare(CarrierDemand {
        countries: 3,
        ..Default::default()
    });
    bases.rows[0] = CarrierBase {
        base: 17,
        countries: Ok((0, 3)),
        host_name: Ok(Some((0, 0))),
    };
    bases.countries[..3].copy_from_slice(&[
        CountryOccurrence {
            country: 8,
            slot: 2,
        },
        CountryOccurrence {
            country: 0,
            slot: 0,
        },
        CountryOccurrence {
            country: 8,
            slot: 2,
        },
    ]);
    let mut registrations = Registrations::new(1);
    registrations.prepare(CarrierDemand {
        pools: 3,
        ..Default::default()
    });
    registrations.rows[0] = CarrierRegistration {
        base: 17,
        slot: 2,
        country: 8,
        fuel: ContainerFuel {
            demand: Err("one guarded scalar failed"),
            allocated: Ok(-9),
        },
        pools: Ok((0, 3)),
    };
    registrations.pools[..3].copy_from_slice(&[
        CarrierPool {
            type_index: -13,
            subunit_index: 2,
            wings_start: 0,
            wings_len: 2,
            planes_raw: Some(-6),
            fuel: CarrierPoolFuel {
                current: None,
                maximum: Some(0),
                ..Default::default()
            },
        },
        CarrierPool {
            type_index: i32::MIN,
            subunit_index: 0,
            wings_start: 2,
            wings_len: 0,
            planes_raw: Some(0),
            fuel: CarrierPoolFuel {
                current: Some(0),
                maximum: Some(0),
                ..Default::default()
            },
        },
        CarrierPool {
            type_index: 6,
            subunit_index: 1,
            wings_start: 2,
            wings_len: 1,
            planes_raw: None,
            fuel: CarrierPoolFuel {
                current: Some(-6),
                maximum: None,
                ..Default::default()
            },
        },
    ]);
    bases.install(lengths(), CarrierDemand::default());
    registrations.install(lengths(), CarrierDemand::default());
    bases.index_rows();
    registrations.index_rows();
    (bases, registrations)
}

fn source() -> cair_base::RegistrationRow {
    cair_base::RegistrationRow {
        base: 17,
        slot: 2,
        capacity_raw: 42,
        // Relative pool offsets must not be compared with this global arena offset.
        wings_start: 37,
        wings_len: 3,
    }
}

#[test]
fn conversion_retains_dense_repeats_empty_text_and_independent_signed_facts() {
    let (bases, registrations) = outputs();
    let rows::Recorded::Value(Some(base)) = bases.packet(17) else {
        panic!("observed base missing")
    };
    assert_eq!(
        base.registered_countries.unwrap(),
        [
            rows::AirBaseCountryOccurrence {
                country: 8,
                slot: 2
            },
            rows::AirBaseCountryOccurrence {
                country: 0,
                slot: 0
            },
            rows::AirBaseCountryOccurrence {
                country: 8,
                slot: 2
            },
        ]
    );
    assert_eq!(base.host_name, Some(Vec::new()));
    let packet = registrations.packet(&source());
    let rows::Recorded::Value(Some(row)) = &packet else {
        panic!("observed registration missing")
    };
    assert_eq!(
        (row.country, row.fuel_demand_raw, row.fuel_allocated_raw),
        (8, None, Some(-9))
    );
    let pools = row.pools.as_ref().unwrap();
    assert_eq!(
        (
            pools[0].wing_start,
            pools[1].wing_start,
            pools[1].wing_count,
            pools[2].wing_start
        ),
        (0, 2, 0, 2)
    );
    assert_eq!((pools[1].subunit_index, pools[1].type_index), (0, i32::MIN));
    assert_eq!(
        (pools[0].current_fuel_raw, pools[0].maximum_fuel_raw),
        (None, Some(0))
    );
    assert!(registrations.equal(&source(), &packet));
}

#[test]
fn malformed_pool_partition_withholds_only_pool_detail() {
    for case in 0..4 {
        let (_, mut registrations) = outputs();
        match case {
            0 => registrations.pools[0].wings_start = 1,
            1 => registrations.pools[2].wings_start = usize::MAX,
            2 => registrations.pools[2].wings_len = 2,
            3 => registrations.rows[0].pools = Ok((0, 2)),
            _ => unreachable!(),
        }
        let rows::Recorded::Value(Some(row)) = registrations.packet(&source()) else {
            panic!("scalar packet erased")
        };
        assert_eq!(
            (row.country, row.fuel_demand_raw, row.fuel_allocated_raw),
            (8, None, Some(-9))
        );
        assert_eq!(row.pools, None, "case {case}");
    }
}

#[test]
fn duplicate_outer_keys_refuse_without_reordering_inner_payloads() {
    let (mut bases, mut registrations) = outputs();
    bases.prepare(CarrierDemand {
        bases: 2,
        ..Default::default()
    });
    bases.rows[1] = bases.rows[0];
    bases.len = 2;
    bases.index_rows();
    registrations.prepare(CarrierDemand {
        registrations: 2,
        ..Default::default()
    });
    registrations.rows[1] = registrations.rows[0];
    registrations.len = 2;
    registrations.index_rows();
    assert_eq!(bases.packet(17), rows::Recorded::Value(None));
    assert_eq!(registrations.packet(&source()), rows::Recorded::Value(None));
    assert_eq!(bases.countries[0].country, 8);
    assert_eq!(registrations.pools[1].wings_len, 0);
}

#[test]
fn current_optional_input_and_refusal_changes_prevent_stale_numeric_memo_restore() {
    let (bases, registrations) = outputs();
    let mut log = RawLog::<2>::new(4, 32);
    log.record(0x5000, 8, Some(&1_i64.to_le_bytes()), 2)
        .unwrap();
    let mut memo = Memo::default();
    assert!(memo.save(&bases, &registrations, &log, CarrierDemand::default()));
    assert!(memo.input_equal(&log));
    log.clear();
    log.record(0x5000, 8, Some(&0_i64.to_le_bytes()), 2)
        .unwrap();
    assert!(
        !memo.input_equal(&log),
        "a genuine current zero must replace the previous numeric result"
    );
    log.clear();
    let _ = log.record(0x5000, 8, None, 2);
    assert!(
        !memo.input_equal(&log),
        "a fresh failure cannot reuse the previous positive input"
    );
    log.clear();
    log.record(0x5000, 8, Some(&1_i64.to_le_bytes()), 2)
        .unwrap();
    log.fault[0] = true;
    assert!(
        !memo.input_equal(&log),
        "requesting-consumer refusal metadata participates in the witness"
    );
}

#[test]
fn optional_fault_and_later_overflow_grow_without_doubling_mandatory_storage() {
    let mut parent = super::super::AirBasesJob::new();
    let predecessor = (
        parent.log.spans.capacity(),
        parent.log.bytes.capacity(),
        parent.census.capacity(),
        parent.seen_links.capacity(),
    );
    parent.carrier.log = RefCell::new(RawLog::new(4, 8));
    {
        let log = parent.carrier.log.get_mut();
        let _ = log.record(0x6000, 8, None, 1);
        log.reserve(32);
        assert!(log.overflowed);
        assert!(
            !log.grow_for_next(),
            "the original raw policy excludes faulted growth"
        );
    }
    assert!(
        parent.grow_for_next(),
        "the optional job must retain its independent pressure"
    );
    parent.grow();
    assert!(parent.carrier.log.borrow().bytes.capacity() >= 32);
    assert_eq!(
        predecessor,
        (
            parent.log.spans.capacity(),
            parent.log.bytes.capacity(),
            parent.census.capacity(),
            parent.seen_links.capacity()
        )
    );
}

#[test]
fn rejected_seed_cannot_admit_its_partial_prefix_on_decode() {
    use super::super::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let mut job = super::super::AirBasesJob::fixture();
    super::super::copy_air_bases_pass(
        &mut job,
        &kit::memory(&bytes),
        kit::BASE,
        kit::GAME_STATE,
        false,
        &mut [0; 3],
    );
    // Keep the readable partial carrier prefix but revoke seed admission.
    job.carrier.seed = Err("carrier census repeated observation changed");
    let mut air = super::super::Captured::with_capacities(&super::super::Capacities::fixture());
    let mut fog = crate::extended::fog::Captured::with_capacities(
        &crate::extended::fog::Capacities::default(),
    );
    air.bases_raw_mode = true;
    fog.registrations_raw_mode = true;
    let mut memo = super::super::AirBasesMemo::default();
    memo.decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 1,
            base: kit::BASE,
            seq: 1,
            hour: 1,
            force_full: true,
        },
        &mut [0; 3],
    );
    assert!(matches!(air.bases.outcome, Outcome::Rows { .. }));
    assert!(fog.registrations_memo_context().is_none());
    assert_eq!(air.carrier.len, 0);
    assert_eq!(fog.carrier.len, 0);
}

#[test]
fn inconsistent_output_lengths_cannot_certify_optional_prefixes() {
    let (mut bases, mut registrations) = outputs();
    bases.install(
        CarrierLengths {
            countries: bases.countries.len() + 1,
            ..lengths()
        },
        CarrierDemand::default(),
    );
    registrations.install(
        CarrierLengths {
            pools: registrations.pools.len() + 1,
            ..lengths()
        },
        CarrierDemand::default(),
    );
    bases.index_rows();
    registrations.index_rows();
    assert_eq!(bases.packet(17), rows::Recorded::Value(None));
    assert_eq!(registrations.packet(&source()), rows::Recorded::Value(None));
}

const COHERENT_HOST_BASE: usize = 0x42000;
const COHERENT_HOST_HANDLE: u64 = (0x43_u64 << 32) | engine::cair_base::TYPE as u64;
const COHERENT_INDEXED: usize = 0x74000;
const COHERENT_DENSE: usize = 0x74400;
const COHERENT_CONTAINER_A: usize = 0x74800;
const COHERENT_CONTAINER_B: usize = 0x74a00;
const COHERENT_POOLS: usize = 0x74c00;
const COHERENT_POOL: usize = 0x74d00;
const COHERENT_OWNER: usize = 0x74e00;
const COHERENT_DATABASE: usize = 0x74f00;
const COHERENT_DEFINITIONS: usize = 0x75000;
const COHERENT_CLONE: usize = 0x75200;
const COHERENT_GLOBAL: usize = 0x75400;
const COHERENT_NULL: usize = 0x75800;
const COHERENT_SHIP: usize = 0x77000;
const COHERENT_NAME: usize = 0x78000;
const COHERENT_REGISTRY: usize = 0x79000;
const COHERENT_BUCKETS: usize = 0x79100;
const COHERENT_SHIP_HANDLE: u64 = (3_u64 << 32) | engine::cship::HANDLE_TYPE as u64;
const COHERENT_NAME_BYTES: &[u8] = b"Deck fixture";

struct CoherentCarrierMemory<'a> {
    bytes: &'a [u8],
    globals: std::collections::BTreeMap<usize, u8>,
    fault: Option<usize>,
    fault_reads: std::cell::Cell<usize>,
}

impl<'a> CoherentCarrierMemory<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let mut globals = std::collections::BTreeMap::new();
        // Only genuine module globals lie outside the existing kit's address
        // space. Their selected objects and payloads use ordinary kit bytes.
        for (address, pointer) in [
            (
                engine::csub_unit_definition::DATABASE_RVA,
                COHERENT_DATABASE,
            ),
            (engine::csub_unit_definition::NULL_OBJECT_RVA, COHERENT_NULL),
            (
                engine::creference_registry::LOW_ROOTS_RVA
                    + engine::cship::HANDLE_TYPE as usize * 8,
                COHERENT_REGISTRY,
            ),
        ] {
            globals.extend(
                (pointer as u64)
                    .to_le_bytes()
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (address + index, value)),
            );
        }
        Self {
            bytes,
            globals,
            fault: None,
            fault_reads: std::cell::Cell::new(0),
        }
    }

    fn with_fault(bytes: &'a [u8], address: usize) -> Self {
        let mut memory = Self::new(bytes);
        memory.fault = Some(address);
        memory
    }
}

impl engine::Memory for CoherentCarrierMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.fault.is_some_and(|fault| {
            address <= fault
                && address
                    .checked_add(destination.len())
                    .is_some_and(|end| fault < end)
        }) {
            self.fault_reads.set(self.fault_reads.get() + 1);
            return false;
        }
        if (0..destination.len()).all(|index| self.globals.contains_key(&(address + index))) {
            for (index, byte) in destination.iter_mut().enumerate() {
                *byte = self.globals[&(address + index)];
            }
            return true;
        }
        super::super::air_bases_test_kit::memory(self.bytes).copy(address, destination)
    }
}

fn coherent_header(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    use super::super::air_bases_test_kit as kit;
    kit::put64(bytes, address, data as u64);
    kit::put32(bytes, address + 8, count);
    kit::put32(bytes, address + 12, count);
}

fn coherent_carrier_fixture() -> Vec<u8> {
    use super::super::air_bases_test_kit as kit;
    use engine::{
        cair_base, cair_wing_pool, ccountry_air_container as container, cname_group_member,
        creference_registry as registry, cship, cstrategic_air, csub_unit_definition as definition,
        reference_bucket as bucket,
    };
    let mut bytes = kit::fixture();
    coherent_header(
        &mut bytes,
        COHERENT_HOST_BASE + cair_base::COUNTRY_CONTAINERS,
        COHERENT_INDEXED,
        3,
    );
    kit::put64(&mut bytes, COHERENT_INDEXED, 0);
    kit::put64(
        &mut bytes,
        COHERENT_INDEXED + 8,
        COHERENT_CONTAINER_A as u64,
    );
    kit::put64(
        &mut bytes,
        COHERENT_INDEXED + 16,
        COHERENT_CONTAINER_B as u64,
    );
    coherent_header(
        &mut bytes,
        COHERENT_HOST_BASE + cair_base::REGISTERED_CONTAINERS,
        COHERENT_DENSE,
        3,
    );
    for (index, pointer) in [
        COHERENT_CONTAINER_B,
        COHERENT_CONTAINER_A,
        COHERENT_CONTAINER_B,
    ]
    .into_iter()
    .enumerate()
    {
        kit::put64(&mut bytes, COHERENT_DENSE + index * 8, pointer as u64);
    }
    for (pointer, country, demand, allocated) in [
        (COHERENT_CONTAINER_A, 1_u32, -7_i64, 11_i64),
        (COHERENT_CONTAINER_B, 2, 23, -29),
    ] {
        kit::put64(&mut bytes, pointer, container::VTABLE_RVA as u64);
        kit::put32(&mut bytes, pointer + container::COUNTRY, country);
        kit::put64(&mut bytes, pointer + container::CAPACITY_RAW, 100000);
        kit::put64(
            &mut bytes,
            pointer + container::FUEL_DEMAND_RAW,
            demand as u64,
        );
        kit::put64(
            &mut bytes,
            pointer + container::FUEL_ALLOCATED_RAW,
            allocated as u64,
        );
    }
    // A real, empty registered pool still has a definition, owner, base,
    // copied wing header and known-zero passive sums. Container B has no pool.
    coherent_header(
        &mut bytes,
        COHERENT_CONTAINER_A + container::POOLS,
        COHERENT_POOLS,
        1,
    );
    coherent_header(&mut bytes, COHERENT_CONTAINER_B + container::POOLS, 0, 0);
    kit::put64(&mut bytes, COHERENT_POOLS, COHERENT_POOL as u64);
    kit::put64(&mut bytes, COHERENT_POOL, cair_wing_pool::VTABLE_RVA as u64);
    kit::put64(
        &mut bytes,
        COHERENT_POOL + cair_wing_pool::AIR_BASE_REFERENCE,
        COHERENT_HOST_HANDLE,
    );
    kit::put64(
        &mut bytes,
        COHERENT_POOL + cair_wing_pool::OWNER,
        COHERENT_OWNER as u64,
    );
    kit::put64(
        &mut bytes,
        COHERENT_POOL + cair_wing_pool::DEFINITION,
        COHERENT_CLONE as u64,
    );
    coherent_header(&mut bytes, COHERENT_POOL + cair_wing_pool::WINGS, 0, 0);
    kit::put64(
        &mut bytes,
        COHERENT_OWNER,
        cstrategic_air::VTABLE_RVA as u64,
    );
    kit::put32(&mut bytes, COHERENT_OWNER + cstrategic_air::COUNTRY, 1);
    coherent_header(
        &mut bytes,
        COHERENT_DATABASE + definition::DATABASE_VECTOR,
        COHERENT_DEFINITIONS,
        2,
    );
    kit::put64(&mut bytes, COHERENT_DEFINITIONS, COHERENT_NULL as u64);
    kit::put64(&mut bytes, COHERENT_DEFINITIONS + 8, COHERENT_GLOBAL as u64);
    for pointer in [COHERENT_CLONE, COHERENT_GLOBAL] {
        kit::put64(&mut bytes, pointer, definition::VTABLE_RVA as u64);
        kit::put32(&mut bytes, pointer + definition::TOKEN, 987);
        bytes[pointer + definition::IS_REAL] = 1;
        kit::put32(&mut bytes, pointer + definition::DATABASE_INDEX, 1);
    }
    kit::put32(&mut bytes, COHERENT_CLONE + 0x60, (-13_i32) as u32);
    kit::put64(
        &mut bytes,
        COHERENT_NULL,
        definition::NULL_VTABLE_RVA as u64,
    );
    kit::put32(
        &mut bytes,
        COHERENT_NULL + definition::TOKEN,
        definition::NULL_TOKEN,
    );
    bytes[COHERENT_NULL + definition::IS_REAL] = 0;

    // Exact type/id registry route and retained SSO name; no generator or UI
    // setting participates. Its name demand is observed before the warm growth.
    kit::put64(
        &mut bytes,
        COHERENT_REGISTRY + registry::ENTRIES,
        COHERENT_BUCKETS as u64,
    );
    kit::put32(&mut bytes, COHERENT_REGISTRY + registry::COUNT, 1);
    kit::put32(&mut bytes, COHERENT_REGISTRY + registry::MASK, 7);
    bytes[COHERENT_REGISTRY + registry::OVERFLOW_DISTANCE] = 5;
    bytes[COHERENT_BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
    let hash = COHERENT_SHIP_HANDLE
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
    let entry = COHERENT_BUCKETS + slot * bucket::STRIDE;
    bytes[entry + bucket::DISTANCE] = 1;
    kit::put64(&mut bytes, entry + bucket::TYPE, COHERENT_SHIP_HANDLE);
    kit::put64(&mut bytes, entry + bucket::OBJECT, COHERENT_SHIP as u64);
    kit::put64(&mut bytes, COHERENT_SHIP, cship::VTABLE_RVA as u64);
    kit::put64(
        &mut bytes,
        COHERENT_SHIP + cship::HANDLE,
        COHERENT_SHIP_HANDLE,
    );
    kit::put64(
        &mut bytes,
        COHERENT_SHIP + cship::NAME_MEMBER,
        COHERENT_NAME as u64,
    );
    kit::put64(
        &mut bytes,
        COHERENT_NAME,
        cname_group_member::VTABLE_RVA as u64,
    );
    let text = COHERENT_NAME + cname_group_member::CACHED_NAME;
    kit::put64(
        &mut bytes,
        text + engine::msvc_string::LENGTH,
        COHERENT_NAME_BYTES.len() as u64,
    );
    kit::put64(
        &mut bytes,
        text + engine::msvc_string::CAPACITY,
        engine::msvc_string::INLINE_CAPACITY,
    );
    bytes[text..text + COHERENT_NAME_BYTES.len()].copy_from_slice(COHERENT_NAME_BYTES);
    bytes
}

fn coherent_carrier_parts() -> (super::super::Captured, crate::extended::fog::Captured) {
    let mut air = super::super::Captured::with_capacities(&super::super::Capacities {
        rows: [0, 8, 0, 8, 0, 4096, 1024, 64, 0],
        ace_names: 32,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
    });
    let mut fog =
        crate::extended::fog::Captured::with_capacities(&crate::extended::fog::Capacities {
            rows: [0, 0, 0, 0, 0, 8, 0],
            radar_provinces: 0,
            area_provinces: 0,
            wings: 64,
        });
    air.bases_raw_mode = true;
    fog.registrations_raw_mode = true;
    (air, fog)
}

fn coherent_carrier_scale_fixture(countries: usize) -> Vec<u8> {
    use super::super::air_bases_test_kit as kit;
    use engine::{ccountry, ccountry_air_container as container, cgame_state};
    let mut bytes = coherent_carrier_fixture();
    let slots = countries + 1;
    let country_table = bytes.len();
    let links = country_table + slots * 8;
    let indexed = (links + slots * 4 + 7) & !7;
    let dense = indexed + slots * 8;
    let objects = dense + slots * 8;
    let country_stride = (ccountry::ACES + 16 + 15) & !15;
    let containers = objects + countries * country_stride;
    bytes.resize(containers + countries * 0x100, 0);
    coherent_header(
        &mut bytes,
        kit::GAME_STATE + cgame_state::COUNTRIES,
        country_table,
        slots as u32,
    );
    coherent_header(
        &mut bytes,
        kit::GAME_STATE + cgame_state::COUNTRY_LINKS,
        links,
        slots as u32,
    );
    coherent_header(
        &mut bytes,
        COHERENT_HOST_BASE + cair_base::COUNTRY_CONTAINERS,
        indexed,
        slots as u32,
    );
    coherent_header(
        &mut bytes,
        COHERENT_HOST_BASE + cair_base::REGISTERED_CONTAINERS,
        dense,
        slots as u32,
    );
    for country in 1..=countries {
        let object = match country {
            1 => kit::C0,
            2 => 0x21000,
            _ => objects + (country - 1) * country_stride,
        };
        let pointer = match country {
            1 => COHERENT_CONTAINER_A,
            2 => COHERENT_CONTAINER_B,
            _ => containers + (country - 1) * 0x100,
        };
        kit::put64(&mut bytes, country_table + country * 8, object as u64);
        kit::put32(&mut bytes, links + country * 4, country as u32);
        kit::put64(&mut bytes, object, ccountry::VTABLE_RVA as u64);
        kit::put32(&mut bytes, object + ccountry::ID, country as u32);
        if country > 2 {
            coherent_header(&mut bytes, object + ccountry::ACES, 0, 0);
        }
        kit::put64(&mut bytes, indexed + country * 8, pointer as u64);
        kit::put64(&mut bytes, dense + country * 8, pointer as u64);
        if country > 2 {
            kit::put64(&mut bytes, pointer, container::VTABLE_RVA as u64);
            kit::put32(&mut bytes, pointer + container::COUNTRY, country as u32);
            coherent_header(&mut bytes, pointer + container::POOLS, 0, 0);
            kit::put64(&mut bytes, pointer + container::CAPACITY_RAW, 100000);
            kit::put64(
                &mut bytes,
                pointer + container::FUEL_DEMAND_RAW,
                country as u64,
            );
            kit::put64(
                &mut bytes,
                pointer + container::FUEL_ALLOCATED_RAW,
                country as u64,
            );
        }
    }
    // Dense native order retains the duplicate country-2 occurrence before
    // indexed countries 1..N, exercising payload demand beyond registry slots.
    kit::put64(&mut bytes, dense, COHERENT_CONTAINER_B as u64);
    bytes
}

#[test]
fn pooled_live_presizing_admits_first_carrier_packets_at_1500_and_5000_countries() {
    // Sizes from the process-wide country hints; hold them at a fresh attachment's values.
    let _hint = crate::country_scale::hint_guard();
    use super::super::air_bases_test_kit as kit;
    for countries in [1_500, 5_000] {
        let bytes = coherent_carrier_scale_fixture(countries);
        let memory = CoherentCarrierMemory::new(&bytes);
        let hint = cair_base::carrier_demand(&memory, kit::BASE, kit::GAME_STATE).unwrap();
        assert_eq!(hint.failure, None);
        assert!(hint.raw_spans >= countries * 7 + 2);
        assert!(hint.raw_bytes >= countries * 40 + 32);
        assert_eq!(hint.countries, countries + 1);
        // Construct before publishing this world's demand, like jobs pooled
        // at menu attach before a campaign loads; then use the real pool path.
        let cold = [
            super::super::AirBasesJob::fixture(),
            super::super::AirBasesJob::fixture(),
        ];
        crate::country_scale::note_slots(countries + 1);
        crate::country_scale::note_carrier_demand(hint);
        let jobs = crate::decode::presize_and_acquire_carrier_jobs_for_test(cold);
        for mut job in jobs {
            assert!(job.carrier.index.len() >= hint.raw_spans * 2);
            assert!(job.carrier.log.borrow().spans.capacity() >= hint.raw_spans);
            assert!(job.carrier.log.borrow().bytes.capacity() >= hint.raw_bytes);
            coherent_carrier_stage(&mut job, &bytes);
            assert_eq!(job.carrier.demand.failure, None);
            let (mut air, mut fog) = coherent_carrier_parts();
            let report = super::super::AirBasesMemo::default().decode(
                &mut air,
                &mut fog,
                &job,
                super::super::AirBasesMemoContext {
                    owner: 17,
                    base: kit::BASE,
                    seq: 1,
                    hour: 201,
                    force_full: true,
                },
                &mut [0; 3],
            );
            assert!(report.installed);
            assert!(matches!(air.bases.outcome, Outcome::Rows { len: 3 }));
            assert!(matches!(fog.registrations_snapshot().outcome,
                Outcome::Rows { len } if len == countries + 2));
            let rows::Recorded::Value(Some(base)) = air.carrier.packet(COHERENT_HOST_HANDLE) else {
                panic!("first-pass carrier packet missing at {countries} countries");
            };
            assert_eq!(base.host_name.as_deref(), Some(COHERENT_NAME_BYTES));
            let dense = base.registered_countries.unwrap();
            assert_eq!(dense.len(), countries + 1);
            assert_eq!(
                dense
                    .iter()
                    .take(3)
                    .map(|row| row.country)
                    .collect::<Vec<_>>(),
                [2, 1, 2]
            );
            for source in fog
                .registrations_snapshot()
                .rows
                .iter()
                .filter(|source| source.base == COHERENT_HOST_HANDLE)
            {
                let rows::Recorded::Value(Some(packet)) = fog.carrier.packet(source) else {
                    panic!("first-pass carrier registration missing");
                };
                assert!(packet.fuel_demand_raw.is_some());
                assert!(packet.fuel_allocated_raw.is_some());
                assert!(packet.pools.is_some());
            }
            assert!(
                !job.grow_for_next(),
                "first pass must need no warmup growth"
            );
        }
    }
}

struct ChangingCensus<'a> {
    memory: CoherentCarrierMemory<'a>,
    address: usize,
    reads: std::cell::Cell<usize>,
}
impl engine::Memory for ChangingCensus<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if !self.memory.copy(address, output) {
            return false;
        }
        if address == self.address && output.len() == 4 {
            let reads = self.reads.get() + 1;
            self.reads.set(reads);
            if reads == 3 {
                output.copy_from_slice(&0_u32.to_le_bytes());
            }
        }
        true
    }
}

fn seed_refusal_count(reason: &str) -> u64 {
    let status = serde_json::to_value(super::super::carrier_capture_status()).unwrap();
    status
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry[0].as_str() == Some(reason))
        .unwrap()[1]
        .as_u64()
        .unwrap()
}

fn assert_counted_seed_refusal(kind: usize, reason: &'static str) {
    use super::super::air_bases_test_kit as kit;
    let mut bytes = coherent_carrier_scale_fixture(3);
    let country_table = 0x80000;
    let links = country_table + 4 * 8;
    let indexed = links + 4 * 4;
    let dense = indexed + 4 * 8;
    // The third country has no base registration. Its three original census
    // witnesses can disagree without changing the independent mandatory rows.
    kit::put64(&mut bytes, indexed + 3 * 8, 0);
    coherent_header(
        &mut bytes,
        COHERENT_HOST_BASE + cair_base::REGISTERED_CONTAINERS,
        dense,
        3,
    );
    let memory = ChangingCensus {
        memory: CoherentCarrierMemory::new(&bytes),
        address: if kind == 2 { links + 3 * 4 } else { usize::MAX },
        reads: std::cell::Cell::new(0),
    };
    let mut job = super::super::AirBasesJob::fixture();
    match kind {
        0 => job.carrier.index.clear(),
        1 => job.carrier.log = std::cell::RefCell::new(RawLog::new(1, 16)),
        2 => {}
        _ => unreachable!(),
    }
    let before = seed_refusal_count(reason);
    let report = super::super::copy_air_bases_refresh(
        &memory,
        super::super::AirBasesCopyContext {
            hour: 201,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert_eq!(job.log.fault, [false; 3]);
    assert_eq!(job.carrier.seed, Err(reason));
    assert_eq!(job.carrier.demand.failure, Some(reason));
    assert_eq!(seed_refusal_count(reason), before + 1);
    if kind < 2 {
        assert!(
            job.carrier.grow_for_next(),
            "seed refusal retains growth pressure"
        );
    }
    let source_spans = job.log.spans.clone();
    let source_bytes = job.log.bytes.as_ref().to_vec();
    let (mut air, mut fog) = coherent_carrier_parts();
    let report = super::super::AirBasesMemo::default().decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq: 1,
            hour: 201,
            force_full: true,
        },
        &mut [0; 3],
    );
    assert!(!report.eligible && !report.installed && !report.hit);
    assert!(matches!(air.bases.outcome, Outcome::Rows { len: 3 }));
    let registrations = fog.registrations_snapshot();
    assert!(matches!(registrations.outcome, Outcome::Rows { len: 4 }));
    assert_eq!(air.carrier.demand.failure, Some(reason));
    assert_eq!(fog.carrier.demand.failure, Some(reason));
    assert_eq!(
        air.carrier.packet(COHERENT_HOST_HANDLE),
        rows::Recorded::Value(None)
    );
    for source in registrations
        .rows
        .iter()
        .filter(|source| source.base == COHERENT_HOST_HANDLE)
    {
        assert_eq!(fog.carrier.packet(source), rows::Recorded::Value(None));
    }
    assert_eq!(
        seed_refusal_count(reason),
        before + 1,
        "owned decode does not count twice"
    );
    assert_eq!(job.log.spans, source_spans);
    assert_eq!(job.log.bytes.as_ref(), source_bytes);
}

#[test]
fn short_carrier_seed_index_is_named_counted_once_and_keeps_mandatory_rows() {
    assert_counted_seed_refusal(0, "carrier exact index capacity");
}

#[test]
fn short_carrier_seed_raw_arena_is_named_counted_once_and_keeps_mandatory_rows() {
    assert_counted_seed_refusal(1, "carrier census seed capacity");
}

#[test]
fn changed_carrier_census_observation_is_named_counted_once_and_keeps_mandatory_rows() {
    assert_counted_seed_refusal(2, "carrier census repeated observation changed");
}

fn coherent_carrier_stage(job: &mut super::super::AirBasesJob, bytes: &[u8]) {
    use super::super::air_bases_test_kit as kit;
    super::super::copy_air_bases_pass(
        job,
        &CoherentCarrierMemory::new(bytes),
        kit::BASE,
        kit::GAME_STATE,
        false,
        &mut [0; 3],
    );
    assert!(!job.log.overflowed);
    assert_eq!(job.log.fault, [false; 3]);
    assert!(job.carrier.seed.is_ok());
    assert!(!job.carrier.log.borrow().overflowed);
    assert_eq!(job.carrier.log.borrow().fault, [false; 2]);
}

#[test]
fn attach_hint_captures_dense_country_and_name_packets_without_a_warmup() {
    use super::super::air_bases_test_kit as kit;
    let bytes = coherent_carrier_fixture();
    let memory = CoherentCarrierMemory::new(&bytes);
    let hint = cair_base::carrier_demand(&memory, kit::BASE, kit::GAME_STATE).unwrap();
    assert_eq!(hint.failure, None);
    assert_eq!(hint.countries, 3);
    assert_eq!(hint.name_bytes, COHERENT_NAME_BYTES.len());
    let mut job = super::super::AirBasesJob::fixture();
    job.carrier.presize(hint);
    coherent_carrier_stage(&mut job, &bytes);
    let (mut air, mut fog) = coherent_carrier_parts();
    let report = super::super::AirBasesMemo::default().decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq: 1,
            hour: 201,
            force_full: true,
        },
        &mut [0; 3],
    );
    assert!(report.installed);
    let rows::Recorded::Value(Some(base)) = air.carrier.packet(COHERENT_HOST_HANDLE) else {
        panic!("first-pass carrier packet missing");
    };
    assert_eq!(base.host_name.as_deref(), Some(COHERENT_NAME_BYTES));
    assert_eq!(base.registered_countries.unwrap().len(), 3);
}

#[test]
fn native_carrier_packets_and_memo_follow_current_fuel_word_changes() {
    use super::super::air_bases_test_kit as kit;
    let mut bytes = coherent_carrier_fixture();
    let mut job = super::super::AirBasesJob::fixture();
    // The first native pass has empty optional arenas. It discovers exact
    // live pressure; satisfy it off-thread BEFORE the accepted capture below.
    coherent_carrier_stage(&mut job, &bytes);
    assert_eq!(job.carrier.demand.countries, 3);
    assert_eq!(job.carrier.demand.indexed, 3);
    assert_eq!(job.carrier.demand.name_bytes, COHERENT_NAME_BYTES.len());
    assert_eq!(
        (
            job.carrier.demand.bases,
            job.carrier.demand.registrations,
            job.carrier.demand.pools
        ),
        (1, 2, 1)
    );
    assert_eq!(job.carrier.demand.failure, None);
    assert!(job.grow_for_next());
    job.grow();

    let mut memo = super::super::AirBasesMemo::default();
    coherent_carrier_stage(&mut job, &bytes);
    let mandatory_spans = job.log.spans.clone();
    let mandatory_bytes = job.log.bytes.as_ref().to_vec();
    let mandatory_bulk = job.log.has_bulk;
    let (mut air, mut fog) = coherent_carrier_parts();
    let first = memo.decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq: 1,
            hour: 201,
            force_full: false,
        },
        &mut [0; 3],
    );
    assert!(first.eligible && first.installed && !first.hit);
    assert!(matches!(air.bases.outcome, Outcome::Rows { len: 3 }));
    assert!(
        air.bases.rows[..3]
            .iter()
            .any(|source| source.id == COHERENT_HOST_HANDLE)
    );
    let first_base = air.carrier.packet(COHERENT_HOST_HANDLE);
    let rows::Recorded::Value(Some(base)) = &first_base else {
        panic!("native base suffix missing")
    };
    assert_eq!(
        base.registered_countries.as_deref(),
        Some(
            [
                rows::AirBaseCountryOccurrence {
                    country: 2,
                    slot: 2
                },
                rows::AirBaseCountryOccurrence {
                    country: 1,
                    slot: 1
                },
                rows::AirBaseCountryOccurrence {
                    country: 2,
                    slot: 2
                },
            ]
            .as_slice()
        )
    );
    assert_eq!(base.host_name.as_deref(), Some(COHERENT_NAME_BYTES));
    let snapshot = fog.registrations_snapshot();
    assert!(matches!(snapshot.outcome, Outcome::Rows { len: 4 }));
    let source = *snapshot
        .rows
        .iter()
        .find(|source| source.base == COHERENT_HOST_HANDLE && source.slot == 1)
        .unwrap();
    let first_registration = fog.carrier.packet(&source);
    let rows::Recorded::Value(Some(registration)) = &first_registration else {
        panic!("native registration suffix missing")
    };
    assert_eq!(
        (
            registration.country,
            registration.fuel_demand_raw,
            registration.fuel_allocated_raw
        ),
        (1, Some(-7), Some(11))
    );
    assert_eq!(
        registration.pools.as_deref(),
        Some(
            [rows::CarrierAirPool {
                type_index: -13,
                subunit_index: 1,
                wing_start: 0,
                wing_count: 0,
                current_fuel_raw: Some(0),
                maximum_fuel_raw: Some(0)
            },]
            .as_slice()
        )
    );

    // Only the two fuel words change. They ride in the original pass's own
    // container span, so its spans stay those of the accepted capture while
    // its bytes change and the memo decodes afresh; the observer reads
    // neither word, so its own log is unchanged.
    kit::put64(
        &mut bytes,
        COHERENT_CONTAINER_A + engine::ccountry_air_container::FUEL_DEMAND_RAW,
        (-31_i64) as u64,
    );
    kit::put64(
        &mut bytes,
        COHERENT_CONTAINER_A + engine::ccountry_air_container::FUEL_ALLOCATED_RAW,
        (-37_i64) as u64,
    );
    coherent_carrier_stage(&mut job, &bytes);
    assert_eq!(job.log.spans, mandatory_spans);
    assert_ne!(job.log.bytes.as_ref(), mandatory_bytes.as_slice());
    assert_eq!(job.log.has_bulk, mandatory_bulk);
    assert!(
        !memo.input_equal(&job.log),
        "the fuel words ride in the original container span"
    );
    assert!(
        memo.carrier.input_equal(&job.carrier.log.borrow()),
        "the observer reads no fuel word"
    );
    air.bases_raw_mode = true;
    fog.registrations_raw_mode = true;
    let changed = memo.decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq: 2,
            hour: 202,
            force_full: false,
        },
        &mut [0; 3],
    );
    assert!(changed.eligible && changed.compared && changed.installed && !changed.hit);
    assert_eq!(air.carrier.packet(COHERENT_HOST_HANDLE), first_base);
    let changed_registration = fog.carrier.packet(&source);
    let rows::Recorded::Value(Some(registration)) = &changed_registration else {
        panic!("current registration suffix missing")
    };
    assert_eq!(
        (
            registration.country,
            registration.fuel_demand_raw,
            registration.fuel_allocated_raw
        ),
        (1, Some(-31), Some(-37))
    );
    assert_ne!(changed_registration, first_registration);
    assert_eq!(fog.registrations_snapshot().rows, snapshot.rows);

    // A third adjacent capture reacquires identical native inputs before the
    // genuine memo hit restores the exact base and UPDATED registration packet.
    coherent_carrier_stage(&mut job, &bytes);
    air.bases_raw_mode = true;
    fog.registrations_raw_mode = true;
    let stable = memo.decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq: 3,
            hour: 203,
            force_full: false,
        },
        &mut [0; 3],
    );
    assert!(stable.eligible && stable.compared && stable.hit);
    assert_eq!(air.carrier.packet(COHERENT_HOST_HANDLE), first_base);
    assert_eq!(fog.carrier.packet(&source), changed_registration);
    assert_eq!(fog.registrations_snapshot().rows, snapshot.rows);
}

fn coherent_carrier_refresh(
    job: &mut super::super::AirBasesJob,
    bytes: &[u8],
    hour: i32,
    force_full: bool,
) {
    use super::super::air_bases_test_kit as kit;
    let report = super::super::copy_air_bases_refresh(
        &CoherentCarrierMemory::new(bytes),
        super::super::AirBasesCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour,
            force_full,
        },
        job,
    );
    assert!(report.raw_ok);
    assert_eq!(job.log.fault, [false; 3]);
    assert!(job.carrier.seed.is_ok());
    assert!(!job.carrier.log.borrow().overflowed);
    assert_eq!(job.carrier.log.borrow().fault, [false; 2]);
}

fn coherent_assert_complete_carrier_packets(
    air: &super::super::Captured,
    fog: &crate::extended::fog::Captured,
) {
    assert!(matches!(air.bases.outcome, Outcome::Rows { len: 3 }));
    let rows::Recorded::Value(Some(base)) = air.carrier.packet(COHERENT_HOST_HANDLE) else {
        panic!("fresh native base suffix missing")
    };
    assert_eq!(
        base.registered_countries.as_deref(),
        Some(
            [
                rows::AirBaseCountryOccurrence {
                    country: 2,
                    slot: 2
                },
                rows::AirBaseCountryOccurrence {
                    country: 1,
                    slot: 1
                },
                rows::AirBaseCountryOccurrence {
                    country: 2,
                    slot: 2
                },
            ]
            .as_slice()
        )
    );
    assert_eq!(base.host_name.as_deref(), Some(COHERENT_NAME_BYTES));
    let snapshot = fog.registrations_snapshot();
    assert!(matches!(snapshot.outcome, Outcome::Rows { len: 4 }));
    for (slot, country, demand, allocated) in [(1, 1, -7, 11), (2, 2, 23, -29)] {
        let source = snapshot
            .rows
            .iter()
            .find(|source| source.base == COHERENT_HOST_HANDLE && source.slot == slot)
            .unwrap();
        let rows::Recorded::Value(Some(registration)) = fog.carrier.packet(source) else {
            panic!("fresh native registration suffix missing")
        };
        assert_eq!(
            (
                registration.country,
                registration.fuel_demand_raw,
                registration.fuel_allocated_raw
            ),
            (country, Some(demand), Some(allocated))
        );
        if slot == 1 {
            assert_eq!(
                registration.pools.as_deref(),
                Some(
                    [rows::CarrierAirPool {
                        type_index: -13,
                        subunit_index: 1,
                        wing_start: 0,
                        wing_count: 0,
                        current_fuel_raw: Some(0),
                        maximum_fuel_raw: Some(0)
                    },]
                    .as_slice()
                )
            );
        } else {
            assert_eq!(registration.pools.as_deref(), Some([].as_slice()));
        }
    }
}

#[test]
fn two_queued_carrier_slots_recover_after_real_retirement_at_the_same_paused_hour() {
    // Sizes from the process-wide country hints; hold them at a fresh attachment's values.
    let _hint = crate::country_scale::hint_guard();
    use super::super::air_bases_test_kit as kit;
    const HOUR: i32 = 201;
    let bytes = coherent_carrier_fixture();
    let mut jobs = [
        super::super::AirBasesJob::fixture(),
        super::super::AirBasesJob::fixture(),
    ];
    let mut frames = [coherent_carrier_parts(), coherent_carrier_parts()];
    let mut memo = super::super::AirBasesMemo::default();

    // Both native jobs are already queued at H before either output frame
    // sees pressure. No job or output arena is locally pre-grown here.
    for job in &mut jobs {
        assert_eq!(job.carrier.bases.capacity()[1..], [0, 0]);
        assert_eq!(job.carrier.registrations.capacity()[1], 0);
        assert_eq!(job.carrier.scratch.capacity(), [0; 5]);
        coherent_carrier_refresh(job, &bytes, HOUR, false);
        assert_eq!(
            (
                job.carrier.demand.countries,
                job.carrier.demand.name_bytes,
                job.carrier.demand.pools
            ),
            (3, COHERENT_NAME_BYTES.len(), 1)
        );
        assert!(
            !job.carrier
                .log
                .borrow()
                .spans
                .iter()
                .any(|span| (COHERENT_DENSE..COHERENT_DENSE + 3 * 8).contains(&span.addr))
        );
        assert!(job.grow_for_next());
    }
    for (index, ((air, fog), job)) in frames.iter_mut().zip(&jobs).enumerate() {
        assert_eq!(air.carrier.capacity()[1..], [0, 0]);
        assert_eq!(fog.carrier.capacity()[1], 0);
        assert_eq!(fog.carrier_scratch.capacity(), [0; 5]);
        let report = memo.decode(
            air,
            fog,
            job,
            super::super::AirBasesMemoContext {
                owner: 17,
                base: kit::BASE,
                seq: index as u64 + 1,
                hour: HOUR,
                force_full: false,
            },
            &mut [0; 3],
        );
        assert!(!report.hit);
        let rows::Recorded::Value(Some(base)) = air.carrier.packet(COHERENT_HOST_HANDLE) else {
            panic!("pressure base row missing")
        };
        // Capacity growth on decode cannot invent the dense pointer payload
        // omitted by the cold native certificate arena. The inline name can
        // independently decode from its already-copied cache header.
        assert!(base.registered_countries.is_none());
        assert!(air.carrier.capacity()[1] >= 3);
        assert!(air.carrier.capacity()[2] >= COHERENT_NAME_BYTES.len());
        assert!(fog.carrier.capacity()[1] >= 1);
    }

    jobs = crate::decode::retire_and_reacquire_two_carrier_jobs_for_test(jobs);
    for job in &jobs {
        assert!(job.log.spans.is_empty());
        assert!(job.carrier.log.borrow().spans.is_empty());
        assert_eq!(job.carrier.demand, CarrierDemand::default());
        assert!(job.carrier.seed.is_err());
    }
    // A refresh, rather than passage of a game hour, settles both queued
    // slots. Native copy runs again at H after actual worker pool growth.
    for job in &mut jobs {
        coherent_carrier_refresh(job, &bytes, HOUR, true);
        assert!(!job.grow_for_next());
    }
    for (index, ((air, fog), job)) in frames.iter_mut().zip(&jobs).enumerate() {
        air.bases_raw_mode = true;
        fog.registrations_raw_mode = true;
        assert_eq!(
            air.carrier.capacity()[1..],
            job.carrier.bases.capacity()[1..]
        );
        assert_eq!(
            fog.carrier.capacity()[1],
            job.carrier.registrations.capacity()[1]
        );
        assert_eq!(
            fog.carrier_scratch.capacity(),
            job.carrier.scratch.capacity()
        );
        let report = memo.decode(
            air,
            fog,
            job,
            super::super::AirBasesMemoContext {
                owner: 17,
                base: kit::BASE,
                seq: index as u64 + 3,
                hour: HOUR,
                force_full: true,
            },
            &mut [0; 3],
        );
        assert!(report.eligible && report.installed && !report.hit && !report.compared);
        coherent_assert_complete_carrier_packets(air, fog);
    }
}

#[test]
fn native_later_host_names_survive_an_earlier_registration_header_failure() {
    use super::super::air_bases_test_kit as kit;
    use engine::{cair_base, cstrategic_air_manager};
    const SECOND_BASE: usize = 0x44000;
    const SECOND_HANDLE: u64 = (0x45_u64 << 32) | cair_base::TYPE as u64;
    const HOST_VECTOR: usize = 0x32000;
    let mut bytes = coherent_carrier_fixture();
    coherent_header(
        &mut bytes,
        kit::MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOST_VECTOR,
        2,
    );
    kit::put64(&mut bytes, HOST_VECTOR + 8, SECOND_BASE as u64);
    kit::put64(&mut bytes, SECOND_BASE, cair_base::VTABLE_RVA as u64);
    kit::put64(&mut bytes, SECOND_BASE + cair_base::HANDLE, SECOND_HANDLE);
    kit::put64(
        &mut bytes,
        SECOND_BASE + cair_base::HOST_REFERENCE,
        COHERENT_SHIP_HANDLE,
    );
    kit::put32(
        &mut bytes,
        SECOND_BASE + cair_base::HOST_REFERENCE + 24,
        (-19_i32) as u32,
    );
    kit::put32(&mut bytes, SECOND_BASE + cair_base::HOST_REFERENCE + 28, 1);
    let memory = CoherentCarrierMemory::with_fault(
        &bytes,
        COHERENT_HOST_BASE + cair_base::COUNTRY_CONTAINERS,
    );
    let mut job = super::super::AirBasesJob::fixture();
    super::super::copy_air_bases_pass(
        &mut job,
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        false,
        &mut [0; 3],
    );
    assert_eq!(
        (job.carrier.demand.bases, job.carrier.demand.name_bytes),
        (2, 2 * COHERENT_NAME_BYTES.len())
    );
    assert_eq!(job.log.fault, [false, true, false]);
    assert!(job.grow_for_next());
    job.grow();
    super::super::copy_air_bases_pass(
        &mut job,
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        false,
        &mut [0; 3],
    );
    assert_eq!(
        memory.fault_reads.get(),
        2,
        "exactly one failed native header read per pass"
    );
    assert!(!job.log.overflowed && job.carrier.seed.is_ok());
    assert_eq!(job.log.fault, [false, true, false]);
    assert!(job.carrier.log.borrow().spans.iter().any(|span| span.addr
        == COHERENT_NAME + engine::cname_group_member::CACHED_NAME
        && span.ok));

    let (mut air, mut fog) = coherent_carrier_parts();
    let report = super::super::AirBasesMemo::default().decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq: 1,
            hour: 201,
            force_full: true,
        },
        &mut [0; 3],
    );
    assert!(!report.eligible && !report.installed && !report.hit);
    assert!(matches!(air.bases.outcome, Outcome::Rows { len: 4 }));
    assert!(matches!(
        fog.registrations_snapshot().outcome,
        Outcome::Unavailable { .. }
    ));
    for handle in [COHERENT_HOST_HANDLE, SECOND_HANDLE] {
        assert!(air.bases.rows[..4].iter().any(|source| source.id == handle));
        let rows::Recorded::Value(Some(base)) = air.carrier.packet(handle) else {
            panic!("independent accepted host name erased")
        };
        assert_eq!(base.host_name.as_deref(), Some(COHERENT_NAME_BYTES));
        assert!(base.registered_countries.is_none());
    }
    assert_eq!(
        memory.fault_reads.get(),
        2,
        "owned decode cannot retry native reads"
    );
}

#[test]
fn native_complete_wing_prefix_is_observed_before_a_later_backlink_read_failure() {
    use super::super::air_bases_test_kit as kit;
    use engine::{cair_wing, cair_wing_pool};
    const HOST_WINGS: usize = 0x76000;
    let mut bytes = coherent_carrier_fixture();
    // Move the existing authenticated primary wings to the genuine host pool.
    // Its old state pool is empty so registration reaches the host before the
    // fault; all handles and both subobject identities remain real kit inputs.
    coherent_header(&mut bytes, kit::POOL0 + cair_wing_pool::WINGS, 0, 0);
    coherent_header(
        &mut bytes,
        COHERENT_POOL + cair_wing_pool::WINGS,
        HOST_WINGS,
        2,
    );
    for (index, wing) in [kit::W0, kit::W1].into_iter().enumerate() {
        kit::put64(&mut bytes, HOST_WINGS + index * 8, wing as u64);
        kit::put64(&mut bytes, wing + 8, COHERENT_POOL as u64);
        kit::put64(&mut bytes, wing + cair_wing::POOL, COHERENT_POOL as u64);
    }
    let mut job = super::super::AirBasesJob::fixture();
    let memory = CoherentCarrierMemory::new(&bytes);
    super::super::copy_air_bases_pass(
        &mut job,
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        false,
        &mut [0; 3],
    );
    assert_eq!(job.log.fault, [false; 3]);
    assert!(job.grow_for_next());
    job.grow();
    let faulting = CoherentCarrierMemory::with_fault(&bytes, kit::W1 + cair_wing::POOL);
    super::super::copy_air_bases_pass(
        &mut job,
        &faulting,
        kit::BASE,
        kit::GAME_STATE,
        false,
        &mut [0; 3],
    );
    assert_eq!(faulting.fault_reads.get(), 1);
    assert_eq!(job.log.fault, [false, true, false]);
    assert!(!job.log.overflowed && job.carrier.seed.is_ok());
    {
        let optional = job.carrier.log.borrow();
        assert!(!optional.overflowed);
        assert!(
            optional
                .spans
                .iter()
                .any(|span| span.addr == kit::W0 + cair_wing::MISSION && span.len == 8 && span.ok)
        );
        assert!(
            !optional
                .spans
                .iter()
                .any(|span| span.addr == kit::W1 + cair_wing::MISSION)
        );
    }
    let (mut air, mut fog) = coherent_carrier_parts();
    let report = super::super::AirBasesMemo::default().decode(
        &mut air,
        &mut fog,
        &job,
        super::super::AirBasesMemoContext {
            owner: 17,
            base: kit::BASE,
            seq: 1,
            hour: 201,
            force_full: true,
        },
        &mut [0; 3],
    );
    assert!(!report.eligible && !report.installed && !report.hit);
    assert!(matches!(
        fog.registrations_snapshot().outcome,
        Outcome::Unavailable { .. }
    ));
    let rows::Recorded::Value(Some(base)) = air.carrier.packet(COHERENT_HOST_HANDLE) else {
        panic!("independent host name erased by wing failure")
    };
    assert_eq!(base.host_name.as_deref(), Some(COHERENT_NAME_BYTES));
    assert!(base.registered_countries.is_none());
    let registration = fog.carrier.rows[..fog.carrier.len]
        .iter()
        .find(|row| row.base == COHERENT_HOST_HANDLE && row.slot == 1)
        .unwrap();
    assert!(registration.pools.is_err());
    assert_eq!(
        fog.carrier.pools_len, 0,
        "incomplete membership publishes no pool detail"
    );
    assert_eq!(
        faulting.fault_reads.get(),
        1,
        "owned decode makes no native retry"
    );
}
