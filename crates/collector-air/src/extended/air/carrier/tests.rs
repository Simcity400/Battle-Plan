use super::*;
#[cfg(test)]
use engine_native_air::cair_base::{CarrierPoolFuel, ContainerFuel};

#[cfg(test)]
fn lengths() -> CarrierLengths {
    CarrierLengths {
        bases: 1,
        registrations: 1,
        pools: 3,
        countries: 3,
        names: 0,
    }
}

#[cfg(test)]
fn outputs() -> (Bases, Registrations) {
    let mut bases = Bases::new(1);
    bases
        .prepare(CarrierDemand {
            countries: 3,
            ..Default::default()
        })
        .unwrap();
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
    registrations
        .prepare(CarrierDemand {
            pools: 3,
            ..Default::default()
        })
        .unwrap();
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

#[cfg(test)]
fn source() -> cair_base::RegistrationRow {
    cair_base::RegistrationRow {
        base: 17,
        slot: 2,
        capacity_raw: 42,
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
    bases
        .prepare(CarrierDemand {
            bases: 2,
            ..Default::default()
        })
        .unwrap();
    bases.rows[1] = bases.rows[0];
    bases.len = 2;
    bases.index_rows();
    registrations
        .prepare(CarrierDemand {
            registrations: 2,
            ..Default::default()
        })
        .unwrap();
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
pub const COHERENT_HOST_HANDLE: u64 = (0x43_u64 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;
const COHERENT_INDEXED: usize = 0x74000;
pub const COHERENT_DENSE: usize = 0x74400;
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
const COHERENT_SHIP_HANDLE: u64 = (3_u64 << 32) | engine_native::cship::HANDLE_TYPE as u64;
pub const COHERENT_NAME_BYTES: &[u8] = b"Deck fixture";

struct CoherentCarrierMemory<'a> {
    bytes: &'a [u8],
    globals: std::collections::BTreeMap<usize, u8>,
    fault: Option<usize>,
    fault_reads: std::cell::Cell<usize>,
}

impl<'a> CoherentCarrierMemory<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let mut globals = std::collections::BTreeMap::new();
        for (address, pointer) in [
            (
                engine_native_parts::csub_unit_definition::DATABASE_RVA.0,
                COHERENT_DATABASE,
            ),
            (
                engine_native_parts::csub_unit_definition::NULL_OBJECT_RVA.0,
                COHERENT_NULL,
            ),
            (
                engine_native_parts::creference_registry::LOW_ROOTS_RVA.0
                    + engine_native::cship::HANDLE_TYPE as usize * 8,
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

    #[cfg(test)]
    fn with_fault(bytes: &'a [u8], address: usize) -> Self {
        let mut memory = Self::new(bytes);
        memory.fault = Some(address);
        memory
    }
}

impl engine_native_memory::Memory for CoherentCarrierMemory<'_> {
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

pub fn coherent_carrier_fixture() -> Vec<u8> {
    use super::super::air_bases_test_kit as kit;
    use engine_native_air::{
        cair_base,
        ccountry_air_container as container,
        cstrategic_air,
    };
    use engine_native_parts::{
        cair_wing_pool,
        creference_registry as registry,
        csub_unit_definition as definition,
        reference_bucket as bucket,
    };
    use engine_native::{cname_group_member, cship};
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
        kit::put64(&mut bytes, pointer, container::VTABLE_RVA.0 as u64);
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
    coherent_header(
        &mut bytes,
        COHERENT_CONTAINER_A + container::POOLS,
        COHERENT_POOLS,
        1,
    );
    coherent_header(&mut bytes, COHERENT_CONTAINER_B + container::POOLS, 0, 0);
    kit::put64(&mut bytes, COHERENT_POOLS, COHERENT_POOL as u64);
    kit::put64(
        &mut bytes,
        COHERENT_POOL,
        cair_wing_pool::VTABLE_RVA.0 as u64,
    );
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
        cstrategic_air::VTABLE_RVA.0 as u64,
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
        kit::put64(&mut bytes, pointer, definition::VTABLE_RVA.0 as u64);
        kit::put32(&mut bytes, pointer + definition::TOKEN, 987);
        bytes[pointer + definition::IS_REAL] = 1;
        kit::put32(&mut bytes, pointer + definition::DATABASE_INDEX, 1);
    }
    kit::put32(&mut bytes, COHERENT_CLONE + 0x60, (-13_i32) as u32);
    kit::put64(
        &mut bytes,
        COHERENT_NULL,
        definition::NULL_VTABLE_RVA.0 as u64,
    );
    kit::put32(
        &mut bytes,
        COHERENT_NULL + definition::TOKEN,
        definition::NULL_TOKEN,
    );
    bytes[COHERENT_NULL + definition::IS_REAL] = 0;

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
    kit::put64(&mut bytes, COHERENT_SHIP, cship::VTABLE_RVA.0 as u64);
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
        cname_group_member::VTABLE_RVA.0 as u64,
    );
    let text = COHERENT_NAME + cname_group_member::CACHED_NAME;
    kit::put64(
        &mut bytes,
        text + engine_native_memory::msvc_string::LENGTH,
        COHERENT_NAME_BYTES.len() as u64,
    );
    kit::put64(
        &mut bytes,
        text + engine_native_memory::msvc_string::CAPACITY,
        engine_native_memory::msvc_string::INLINE_CAPACITY,
    );
    bytes[text..text + COHERENT_NAME_BYTES.len()].copy_from_slice(COHERENT_NAME_BYTES);
    bytes
}

pub fn coherent_carrier_parts() -> (super::super::Captured, crate::extended::fog::Captured) {
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

#[cfg(test)]
fn coherent_carrier_scale_fixture(countries: usize) -> Vec<u8> {
    use super::super::air_bases_test_kit as kit;
    use engine_native::{ccountry, cgame_state};
    use engine_native_air::ccountry_air_container as container;
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
        kit::put64(&mut bytes, object, ccountry::VTABLE_RVA.0 as u64);
        kit::put32(&mut bytes, object + ccountry::ID, country as u32);
        if country > 2 {
            coherent_header(&mut bytes, object + ccountry::ACES, 0, 0);
        }
        kit::put64(&mut bytes, indexed + country * 8, pointer as u64);
        kit::put64(&mut bytes, dense + country * 8, pointer as u64);
        if country > 2 {
            kit::put64(&mut bytes, pointer, container::VTABLE_RVA.0 as u64);
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
    kit::put64(&mut bytes, dense, COHERENT_CONTAINER_B as u64);
    bytes
}

#[cfg(test)]
struct ChangingCensus<'a> {
    memory: CoherentCarrierMemory<'a>,
    address: usize,
    reads: std::cell::Cell<usize>,
}
#[cfg(test)]
impl engine_native_memory::Memory for ChangingCensus<'_> {
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

#[cfg(test)]
fn assert_counted_seed_refusal(reason: &'static str) {
    use super::super::air_bases_test_kit as kit;
    let mut bytes = coherent_carrier_scale_fixture(3);
    let country_table = 0x80000;
    let links = country_table + 4 * 8;
    let indexed = links + 4 * 4;
    let dense = indexed + 4 * 8;
    kit::put64(&mut bytes, indexed + 3 * 8, 0);
    coherent_header(
        &mut bytes,
        COHERENT_HOST_BASE + cair_base::REGISTERED_CONTAINERS,
        dense,
        3,
    );
    let memory = ChangingCensus {
        memory: CoherentCarrierMemory::new(&bytes),
        address: links + 3 * 4,
        reads: std::cell::Cell::new(0),
    };
    let mut job = super::super::AirBasesJob::fixture();
    let report = super::super::copy_air_bases_fixture(
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
    assert_eq!(job.log.spans, source_spans);
    assert_eq!(job.log.bytes.as_ref(), source_bytes);
}

#[cfg(test)]
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
    assert!(!job.grow_for_next());

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

    kit::put64(
        &mut bytes,
        COHERENT_CONTAINER_A + engine_native_air::ccountry_air_container::FUEL_DEMAND_RAW,
        (-31_i64) as u64,
    );
    kit::put64(
        &mut bytes,
        COHERENT_CONTAINER_A + engine_native_air::ccountry_air_container::FUEL_ALLOCATED_RAW,
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

#[test]
fn refused_carrier_storage_refuses_both_tables_by_name_and_the_next_hour_records_every_row() {
    use super::super::air_bases_test_kit as kit;
    let bytes = coherent_carrier_fixture();
    let mut job = super::super::AirBasesJob::fixture();
    coherent_carrier_stage(&mut job, &bytes);
    job.grow();
    coherent_carrier_stage(&mut job, &bytes);
    let ctx = |seq, hour| super::super::AirBasesMemoContext {
        owner: 17,
        base: kit::BASE,
        seq,
        hour,
        force_full: false,
    };
    let mut memo = super::super::AirBasesMemo::default();
    let (mut air, mut fog) = coherent_carrier_parts();
    assert!(air.carrier.capacity()[1] < job.carrier.demand.countries);
    let prepared = collector_test_support::with_allocations_denied(|| {
        fog.prepare_decode(&job, &mut air.carrier)
    });
    assert_eq!(prepared, Err("capture buffer allocation failed"));
    let refused =
        memo.decode_prepared(&mut air, &mut fog, &job, ctx(1, 201), &mut [0; 3], prepared);
    assert!(!refused.eligible && !refused.installed && !refused.hit);
    let named = Outcome::Unavailable {
        reason: "capture buffer allocation failed",
    };
    assert_eq!(air.bases.outcome, named);
    assert_eq!(fog.registrations_snapshot().outcome, named);

    air.bases_raw_mode = true;
    fog.registrations_raw_mode = true;
    let recorded = memo.decode(&mut air, &mut fog, &job, ctx(2, 202), &mut [0; 3]);
    assert!(recorded.eligible && recorded.installed && !recorded.hit);
    coherent_assert_complete_carrier_packets(&air, &fog);
}

pub fn coherent_carrier_refresh(
    job: &mut super::super::AirBasesJob,
    bytes: &[u8],
    hour: i32,
    force_full: bool,
) {
    use super::super::air_bases_test_kit as kit;
    let report = super::super::copy_air_bases_fixture(
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

pub fn coherent_assert_complete_carrier_packets(
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
fn native_later_host_names_survive_an_earlier_registration_header_failure() {
    use super::super::air_bases_test_kit as kit;
    use engine_native_air::{cair_base, cstrategic_air_manager};
    const SECOND_BASE: usize = 0x44000;
    const SECOND_HANDLE: u64 = (0x45_u64 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;
    const HOST_VECTOR: usize = 0x32000;
    let mut bytes = coherent_carrier_fixture();
    coherent_header(
        &mut bytes,
        kit::MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOST_VECTOR,
        2,
    );
    kit::put64(&mut bytes, HOST_VECTOR + 8, SECOND_BASE as u64);
    kit::put64(&mut bytes, SECOND_BASE, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
    kit::put64(&mut bytes, SECOND_BASE + engine_native_parts::cair_base_layout::HANDLE, SECOND_HANDLE);
    kit::put64(
        &mut bytes,
        SECOND_BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE,
        COHERENT_SHIP_HANDLE,
    );
    kit::put32(
        &mut bytes,
        SECOND_BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE + 24,
        (-19_i32) as u32,
    );
    kit::put32(&mut bytes, SECOND_BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE + 28, 1);
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
    assert!(!job.grow_for_next());
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
        == COHERENT_NAME + engine_native::cname_group_member::CACHED_NAME
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
    use engine_native_air::cair_wing;
    use engine_native_parts::cair_wing_pool;
    const HOST_WINGS: usize = 0x76000;
    let mut bytes = coherent_carrier_fixture();
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
        kit::put64(&mut bytes, wing + engine_native_parts::cair_wing_layout::POOL, COHERENT_POOL as u64);
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
    assert!(!job.grow_for_next());
    let faulting = CoherentCarrierMemory::with_fault(&bytes, kit::W1 + engine_native_parts::cair_wing_layout::POOL);
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

#[test]
fn quiet_base_projection_reuses_nested_country_order_without_allocating_a_payload() {
    use collector_projection::projection::{Projection, ProjectionScratch};
    use replay_frame::frame::{FrameEncoder, FrameInput, Metrics, TableInput};
    let (bases, _) = outputs();
    let mut captured =
        super::super::Captured::with_capacities(&super::super::Capacities::fixture());
    captured.carrier = bases;
    captured.bases.rows[0] = cair_base::Row {
        id: 17,
        state: Some(1),
        ..Default::default()
    };
    captured.bases.outcome = Outcome::Rows { len: 1 };
    let input = |hour, base| {
        let mut frame = FrameInput {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                collector_projection::table_conversion::unavailable("fixture table").into()
            }),
        };
        frame.tables[TableId::AirBases.index()] = base;
        frame
    };
    let mut encoder = FrameEncoder::new();
    encoder
        .encode_input(input(1, captured.base_input(None)), false)
        .unwrap();
    let scratch = ProjectionScratch::default();
    let mut projection = Projection::new(&encoder, 1, 2, false, &scratch);
    assert!(matches!(
        captured.base_input(Some(&mut projection)),
        TableInput::ComparedEqual(_)
    ));
    collector_test_support::with_allocations_denied(|| {
        assert!(matches!(
            captured.base_input(Some(&mut projection)),
            TableInput::ComparedEqual(_)
        ));
    });
    captured.carrier.countries.swap(0, 1);
    assert!(!matches!(
        captured.base_input(Some(&mut projection)),
        TableInput::ComparedEqual(_)
    ));
}

#[test]
fn state_packets_override_only_their_keys_and_refresh_even_after_a_carrier_memo_restore() {
    let (bases, mut registrations) = outputs();
    let mut memo = Memo::default();
    let log = RawLog::<2>::default();
    assert!(memo.save(&bases, &registrations, &log, CarrierDemand::default()));
    let source = cair_base::RegistrationRow {
        base: 29,
        slot: 3,
        wings_len: 0,
        ..Default::default()
    };
    let mut state = super::super::state_fuel_inputs::Packet::default();
    state.rows.push(CarrierRegistration {
        base: 29,
        slot: 3,
        country: 8,
        pools: Ok((0, 1)),
        fuel: ContainerFuel {
            demand: Ok(-19),
            allocated: Ok(7),
        },
    });
    state.pools.push(CarrierPool {
        type_index: -4,
        subunit_index: 8,
        ..Default::default()
    });
    registrations.install_states(&state).unwrap();
    let packet = registrations.packet(&source);
    let rows::Recorded::Value(Some(ref packet)) = packet else {
        panic!("state packet");
    };
    assert_eq!(packet.fuel_demand_raw, None);
    assert_eq!(registrations.state_fuel(&source), [Some(-19), Some(7)]);
    assert_eq!(packet.pools.as_ref().unwrap()[0].type_index, -4);
    assert_eq!(packet.pools.as_ref().unwrap()[0].current_fuel_raw, None);
    state.rows[0].fuel.allocated = Ok(11);
    let mut restored_bases = Bases::new(0);
    restored_bases
        .prepare(CarrierDemand {
            bases: memo.bases.len(),
            countries: memo.countries.len(),
            ..Default::default()
        })
        .unwrap();
    memo.restore(&mut restored_bases, &mut registrations);
    registrations.install_states(&state).unwrap();
    let rows::Recorded::Value(Some(packet)) = registrations.packet(&source) else {
        panic!("refreshed state packet");
    };
    assert_eq!(packet.fuel_allocated_raw, None);
    assert_eq!(registrations.state_fuel(&source), [Some(-19), Some(11)]);
    registrations
        .install_states(&super::super::state_fuel_inputs::Packet::default())
        .unwrap();
    assert_eq!(registrations.packet(&source), rows::Recorded::Value(None));
}
