use super::*;
use engine_native::cgame_state;
use engine_native_parts::creference_registry as registry;
use std::{cell::RefCell, collections::BTreeMap};

const GAME: usize = 0x10000;
const COUNTRIES: usize = 0x11000;
const LINKS: usize = 0x12000;
const COUNTRY_A: usize = 0x13000;
const COUNTRY_B: usize = 0x14000;
const AIR_BASE: usize = 0x15000;
const DENSE: usize = 0x16000;
const CONTAINER_A: usize = 0x17000;
const CONTAINER_B: usize = 0x18000;
const CONTAINER_ZERO: usize = 0x19000;
const POOL: usize = 0x1a000;
const OWNER: usize = 0x1b000;
const DEFINITION: usize = 0x1c000;
const GLOBAL: usize = 0x1d000;
const DATABASE: usize = 0x1e000;
const DEFINITIONS: usize = 0x1f000;
const SHIP: usize = 0x20000;
const NAME: usize = 0x22000;
const HEAP: usize = 0x23000;
const NULL_DEFINITION: usize = 0x24000;
const SECOND_GLOBAL: usize = 0x25000;
const BASE_HANDLE: u64 = (7 << 32) | crate::cair_base::TYPE as u64;
const HOST_HANDLE: u64 = (19 << 32) | cship::HANDLE_TYPE as u64;

struct Fixture {
    bytes: BTreeMap<usize, u8>,
    registry: registry::test_support::Fixture,
    fault: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes
            .extend(bytes.iter().enumerate().map(|(i, &value)| (at + i, value)));
    }
    fn qword(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn dword(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.qword(at, data as u64);
        self.dword(at + 8, count);
        self.dword(at + 12, count);
    }
    fn definition(&mut self, at: usize, token: u32, real: bool) {
        self.put(at, &[0; 17]);
        self.qword(
            at,
            if real {
                definition::VTABLE_RVA.0
            } else {
                definition::NULL_VTABLE_RVA.0
            } as u64,
        );
        self.dword(at + definition::TOKEN, token);
        self.put(at + definition::IS_REAL, &[u8::from(real)]);
    }
    fn new() -> Self {
        let mut result = Self {
            bytes: BTreeMap::new(),
            registry: registry::test_support::Fixture::new(cship::HANDLE_TYPE),
            fault: None,
            reads: RefCell::default(),
        };
        result.array(GAME + cgame_state::COUNTRIES, COUNTRIES, 3);
        result.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, 10);
        result.put(COUNTRIES, &[0; 3 * 8]);
        result.put(LINKS, &[0; 10 * 4]);
        for (slot, raw, pointer) in [(1usize, 5usize, COUNTRY_A), (2, 9, COUNTRY_B)] {
            result.qword(COUNTRIES + slot * 8, pointer as u64);
            result.dword(LINKS + slot * 4, slot as u32);
            result.dword(LINKS + raw * 4, slot as u32);
            result.qword(pointer, ccountry::VTABLE_RVA.0 as u64);
            result.dword(pointer + ccountry::ID, raw as u32);
        }
        for (pointer, raw) in [(CONTAINER_A, 5), (CONTAINER_B, 9), (CONTAINER_ZERO, 8)] {
            result.qword(pointer, ccountry_air_container::VTABLE_RVA.0 as u64);
            result.dword(pointer + ccountry_air_container::COUNTRY, raw);
        }
        result.array(AIR_BASE + crate::cair_base::REGISTERED_CONTAINERS, DENSE, 3);
        for (index, pointer) in [CONTAINER_B, CONTAINER_A, CONTAINER_B]
            .into_iter()
            .enumerate()
        {
            result.qword(DENSE + index * 8, pointer as u64);
        }
        result.qword(
            CONTAINER_B + ccountry_air_container::FUEL_DEMAND_RAW,
            i64::MIN as u64,
        );
        result.qword(
            CONTAINER_B + ccountry_air_container::FUEL_ALLOCATED_RAW,
            i64::MAX as u64,
        );
        result.qword(POOL + cair_wing_pool::AIR_BASE_REFERENCE, BASE_HANDLE);
        result.qword(POOL + cair_wing_pool::OWNER, OWNER as u64);
        result.qword(OWNER, cstrategic_air::VTABLE_RVA.0 as u64);
        result.dword(OWNER + cstrategic_air::COUNTRY, 2);
        result.qword(POOL + cair_wing_pool::DEFINITION, DEFINITION as u64);
        result.definition(DEFINITION, 987, true);
        result.definition(GLOBAL, 987, true);
        result.dword(DEFINITION + definition::DATABASE_INDEX, 1);
        result.dword(DEFINITION + TYPE_INDEX, i32::MIN as u32);
        result.qword(definition::DATABASE_RVA.0, DATABASE as u64);
        result.array(DATABASE + definition::DATABASE_VECTOR, DEFINITIONS, 2);
        result.qword(DEFINITIONS, NULL_DEFINITION as u64);
        result.qword(DEFINITIONS + 8, GLOBAL as u64);
        result.qword(definition::NULL_OBJECT_RVA.0, NULL_DEFINITION as u64);
        result.definition(NULL_DEFINITION, definition::NULL_TOKEN, false);
        result.dword(NULL_DEFINITION + TYPE_INDEX, (-17i32) as u32);
        result
    }
    fn indexed(&self) -> [Option<IndexedContainer>; 3] {
        [
            Some(IndexedContainer {
                base: BASE_HANDLE,
                pointer: CONTAINER_ZERO,
                country: 8,
                slot: 0,
            }),
            Some(IndexedContainer {
                base: BASE_HANDLE,
                pointer: CONTAINER_A,
                country: 5,
                slot: 1,
            }),
            Some(IndexedContainer {
                base: BASE_HANDLE,
                pointer: CONTAINER_B,
                country: 9,
                slot: 2,
            }),
        ]
    }
    fn host(&mut self, text: &[u8]) {
        self.registry.entry(
            registry::test_support::Fixture::index(HOST_HANDLE),
            1,
            HOST_HANDLE,
            SHIP as u64,
        );
        self.registry
            .put(0x1000 + registry::COUNT, &1u32.to_le_bytes());
        self.qword(SHIP, cship::VTABLE_RVA.0 as u64);
        self.qword(SHIP + cship::HANDLE, HOST_HANDLE);
        self.qword(SHIP + cship::NAME_MEMBER, NAME as u64);
        self.qword(NAME, cname_group_member::VTABLE_RVA.0 as u64);
        let string = NAME + cname_group_member::CACHED_NAME;
        self.put(string, &[0; 32]);
        self.qword(string + 16, text.len() as u64);
        if text.len() <= 15 {
            self.qword(string + 24, 15);
            self.put(string, text);
        } else {
            self.qword(string, HEAP as u64);
            self.qword(string + 24, text.len() as u64);
            self.put(HEAP, text);
            self.put(HEAP + text.len(), &[0]);
        }
    }
}

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self
            .fault
            .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
        {
            return false;
        }
        if (0..destination.len()).all(|index| self.bytes.contains_key(&(address + index))) {
            for (index, byte) in destination.iter_mut().enumerate() {
                *byte = self.bytes[&(address + index)];
            }
            true
        } else {
            self.registry.copy(address, destination)
        }
    }
}

fn dense(
    memory: &Fixture,
    indexed: &[Option<IndexedContainer>],
    rows: &mut Vec<CountryOccurrence>,
) -> (Result<(), &'static str>, usize, usize) {
    let countries = ccountry::Reader::new(memory, 0, GAME).unwrap();
    let mut cells = [None; 10];
    let mut resolved = super::super::Resolver {
        cells: &mut cells,
    };
    let mut output = DenseCountries {
        rows: Buffer::Growing(rows),
        len: usize::MAX,
        required: usize::MAX,
    };
    let result = dense_countries(
        memory,
        AIR_BASE,
        BASE_HANDLE,
        &countries,
        &mut resolved,
        indexed,
        &mut output,
    );
    (result, output.len, output.required)
}

#[test]
fn dense_occurrences_preserve_unsorted_repeats_and_authenticated_reserved_zero() {
    let mut memory = Fixture::new();
    let mut rows = vec![CountryOccurrence::default(); 3];
    assert_eq!(dense(&memory, &memory.indexed(), &mut rows), (Ok(()), 3, 3));
    assert_eq!(
        rows,
        [
            CountryOccurrence {
                country: 9,
                slot: 2
            },
            CountryOccurrence {
                country: 5,
                slot: 1
            },
            CountryOccurrence {
                country: 9,
                slot: 2
            },
        ]
    );
    memory.qword(DENSE + 8, CONTAINER_ZERO as u64);
    assert_eq!(dense(&memory, &memory.indexed(), &mut rows), (Ok(()), 3, 3));
    assert_eq!(
        rows[1],
        CountryOccurrence {
            country: 8,
            slot: 0
        }
    );
    memory.fault = Some(LINKS + 8 * 4);
    assert_eq!(dense(&memory, &memory.indexed(), &mut rows).1, 1);
    assert!(dense(&memory, &memory.indexed(), &mut rows).0.is_err());
}

#[test]
fn dense_missing_slot_wrong_base_and_null_never_become_empty() {
    let mut memory = Fixture::new();
    let mut rows = vec![CountryOccurrence::default(); 3];
    let mut indexed = memory.indexed();
    indexed[2] = None;
    assert_eq!(
        dense(&memory, &indexed, &mut rows),
        (
            Err("carrier dense country has no authenticated indexed registration"),
            0,
            3
        )
    );
    let mut indexed = memory.indexed();
    indexed[2].as_mut().unwrap().base ^= 1u64 << 32;
    assert_eq!(
        dense(&memory, &indexed, &mut rows),
        (
            Err("carrier dense country differs from indexed registration"),
            0,
            3
        )
    );
    memory.qword(DENSE, 0);
    assert_eq!(
        dense(&memory, &memory.indexed(), &mut rows),
        (Err("carrier dense country occurrence is null"), 0, 3)
    );
    memory.array(AIR_BASE + crate::cair_base::REGISTERED_CONTAINERS, 0, 0);
    assert_eq!(dense(&memory, &[], &mut rows), (Ok(()), 0, 0));
}

#[test]
fn the_container_span_holds_capacity_and_both_signed_fuel_scalars() {
    use crate::cair_base::{CONTAINER_SPAN, container_span_words};
    assert_eq!(
        ccountry_air_container::COUNTRY + CONTAINER_SPAN,
        ccountry_air_container::SIZE
    );
    let mut span = [0_u8; CONTAINER_SPAN];
    for (offset, value) in [
        (ccountry_air_container::CAPACITY_RAW, 100_000_i64),
        (ccountry_air_container::FUEL_DEMAND_RAW, i64::MIN),
        (ccountry_air_container::FUEL_ALLOCATED_RAW, i64::MAX),
    ] {
        let at = offset - ccountry_air_container::COUNTRY;
        span[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(
        container_span_words(&span),
        (
            100_000,
            ContainerFuel {
                demand: Ok(i64::MIN),
                allocated: Ok(i64::MAX)
            }
        )
    );
}

fn metadata(memory: &Fixture, count: u32, start: usize) -> Result<PoolMetadata, &'static str> {
    let countries = ccountry::Reader::new(memory, 0, GAME).unwrap();
    let mut cells = [None; 10];
    let mut resolved = super::super::Resolver {
        cells: &mut cells,
    };
    pool_metadata(
        memory,
        0,
        PoolObservation {
            pointer: POOL,
            registration: memory.indexed()[2].unwrap(),
            members: Array {
                data: 0xdead0000,
                count,
            },
            wings_start: start,
        },
        &countries,
        &mut resolved,
        &mut None,
    )
}

#[test]
fn empty_and_nonempty_pools_keep_signed_group_key_token_and_exact_wing_boundaries() {
    let memory = Fixture::new();
    for count in [0, 3] {
        let row = metadata(&memory, count, 17).unwrap();
        assert_eq!((row.wings_start, row.wings_len), (17, count));
        assert_eq!(
            (row.definition.type_index, row.definition.subunit_index),
            (i32::MIN, 1)
        );
        assert_eq!(
            (row.pool, row.owner, row.base_reference),
            (POOL, OWNER, BASE_HANDLE)
        );
        assert_eq!(
            row.members,
            Array {
                data: 0xdead0000,
                count
            }
        );
    }
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at >= 0xdead0000)
    );
    assert_eq!(
        metadata(&memory, 1, usize::MAX),
        Err("carrier pool wing boundary overflow")
    );
}

#[test]
fn pool_base_country_clone_and_null_definition_failures_are_independent() {
    let mut memory = Fixture::new();
    memory.qword(
        POOL + cair_wing_pool::AIR_BASE_REFERENCE,
        BASE_HANDLE ^ (1u64 << 32),
    );
    assert_eq!(
        metadata(&memory, 0, 0),
        Err("carrier registered pool belongs to another base")
    );
    memory.qword(POOL + cair_wing_pool::AIR_BASE_REFERENCE, BASE_HANDLE);
    memory.dword(OWNER + cstrategic_air::COUNTRY, 5);
    assert_eq!(
        metadata(&memory, 0, 0),
        Err("carrier registered pool country differs from its container")
    );
    memory.dword(OWNER + cstrategic_air::COUNTRY, 2);
    memory.dword(GLOBAL + definition::TOKEN, 988);
    assert_eq!(
        metadata(&memory, 0, 0),
        Err("carrier pool definition differs from its content database entry")
    );
    memory.qword(POOL + cair_wing_pool::DEFINITION, NULL_DEFINITION as u64);
    let row = metadata(&memory, 0, 0).unwrap();
    assert_eq!(
        (row.definition.type_index, row.definition.subunit_index),
        (-17, 0)
    );
    memory.qword(DEFINITIONS, DEFINITION as u64);
    assert_eq!(
        metadata(&memory, 0, 0),
        Err("carrier null definition differs from the native content singleton")
    );
    memory.qword(DEFINITIONS, NULL_DEFINITION as u64);
    memory.put(NULL_DEFINITION + definition::IS_REAL, &[2]);
    assert_eq!(
        metadata(&memory, 0, 0),
        Err("carrier pool definition identity mismatch")
    );
}

#[test]
fn equal_or_zero_tokens_never_replace_the_held_definition_content_index() {
    let mut memory = Fixture::new();
    memory.array(DATABASE + definition::DATABASE_VECTOR, DEFINITIONS, 3);
    memory.qword(DEFINITIONS + 16, SECOND_GLOBAL as u64);
    memory.dword(DEFINITION + definition::DATABASE_INDEX, 2);
    for token in [987, 0] {
        memory.definition(DEFINITION, token, true);
        memory.definition(SECOND_GLOBAL, token, true);
        let row = metadata(&memory, 0, 0).unwrap();
        assert_eq!(row.definition.subunit_index, 2);
        assert_eq!(row.definition.type_index, i32::MIN);
    }
}

fn source(host: u64) -> crate::cair_base::Row {
    crate::cair_base::Row {
        id: BASE_HANDLE,
        host,
        ..Default::default()
    }
}

#[test]
fn host_null_and_known_stale_are_empty_but_failed_resolution_is_unknown() {
    let mut memory = Fixture::new();
    let mut names = cname_group_member::Names {
        bytes: engine_native_memory::walk::Buffer::Count,
        len: 0,
        required: 0,
    };
    let mut resolver = registry::ScopedResolver::new(0);
    assert_eq!(
        host_name(&memory, 0, &source(0), &mut resolver, &mut names),
        Ok(Some((0, 0)))
    );
    assert!(memory.reads.borrow().is_empty());
    assert_eq!(
        host_name(&memory, 0, &source(HOST_HANDLE), &mut resolver, &mut names),
        Ok(Some((0, 0)))
    );
    memory.fault = Some(registry::LOW_ROOTS_RVA.0 + cship::HANDLE_TYPE as usize * 8);
    let mut resolver = registry::ScopedResolver::new(0);
    assert!(host_name(&memory, 0, &source(HOST_HANDLE), &mut resolver, &mut names).is_err());
    assert_eq!((names.len, names.required), (0, 0));
}

#[test]
fn retained_host_cache_is_bound_to_full_handle_and_uncommitted_raw_bytes() {
    let text = b"A retained carrier\0name in the current language";
    let mut memory = Fixture::new();
    memory.host(text);
    let mut bytes = vec![0x7f; text.len() + 3];
    let mut names = cname_group_member::Names {
        bytes: engine_native_memory::walk::Buffer::Growing(&mut bytes),
        len: 3,
        required: 0,
    };
    let mut resolver = registry::ScopedResolver::new(0);
    assert_eq!(
        host_name_demand(&memory, 0, &source(HOST_HANDLE), &mut resolver),
        Ok(text.len())
    );
    assert_eq!(
        host_name(&memory, 0, &source(HOST_HANDLE), &mut resolver, &mut names),
        Ok(Some((3, text.len())))
    );
    assert_eq!((names.len, names.required), (3, text.len()));
    assert_eq!(&names.bytes[..3], &[0x7f; 3]);
    assert_eq!(&names.bytes[3..], text);
    memory.qword(SHIP + cship::HANDLE, HOST_HANDLE ^ (1u64 << 32));
    assert!(host_name(&memory, 0, &source(HOST_HANDLE), &mut resolver, &mut names).is_err());
}

#[test]
fn short_host_arena_counts_demand_without_hiding_dense_or_fuel_fields() {
    let mut memory = Fixture::new();
    let text = b"A retained carrier name longer than the admitted arena";
    memory.host(text);
    let mut names = cname_group_member::Names {
        bytes: engine_native_memory::walk::Buffer::Count,
        len: 0,
        required: 0,
    };
    let mut resolver = registry::ScopedResolver::new(0);
    assert_eq!(
        host_name(&memory, 0, &source(HOST_HANDLE), &mut resolver, &mut names),
        Ok(None)
    );
    assert_eq!((names.len, names.required), (0, text.len()));
    let mut rows = vec![CountryOccurrence::default(); 3];
    assert_eq!(dense(&memory, &memory.indexed(), &mut rows), (Ok(()), 3, 3));
}

#[test]
fn growing_carrier_arenas_publish_first_and_later_mod_sized_suffixes() {
    use crate::cair_base::{CarrierCapture, CarrierBuffers};
    let mut memory = Fixture::new();
    let text = vec![b'X'; 65_537];
    memory.host(&text);
    let mut bases = Vec::new();
    let mut registrations = Vec::new();
    let mut pools = Vec::new();
    let mut dense = Vec::new();
    let mut names = Vec::new();
    let mut indexed = Vec::new();
    let mut resolved = Vec::new();
    let mut country_terms = Vec::new();
    let mut aces = Vec::new();
    let mut forecasts = Vec::new();
    for count in [50_u32, 120] {
        let dense_data = 0x600_000;
        memory.array(
            AIR_BASE + crate::cair_base::REGISTERED_CONTAINERS,
            dense_data,
            count,
        );
        for index in 0..count as usize {
            memory.qword(dense_data + index * 8, CONTAINER_B as u64);
        }
        let countries = ccountry::Reader::new(&memory, 0, GAME).unwrap();
        let lengths = {
            let mut capture = CarrierCapture::new(
                &memory,
                0,
                GAME,
                &countries,
                CarrierBuffers {
                    bases: &mut bases,
                    registrations: &mut registrations,
                    pools: &mut pools,
                    countries: &mut dense,
                    names: &mut names,
                    indexed: &mut indexed,
                    resolved: &mut resolved,
                    country_terms: &mut country_terms,
                    aces: &mut aces,
                    forecasts: &mut forecasts,
                },
            );
            capture.begin_base(AIR_BASE, BASE_HANDLE, true);
            capture.authenticated_base(source(HOST_HANDLE));
            capture.indexed_header(3);
            capture.indexed_container(9, 2, CONTAINER_B);
            capture.begin_registration(ContainerFuel {
                demand: Ok(0),
                allocated: Ok(0),
            });
            capture.registration_header(count);
            for _ in 0..count {
                capture.begin_pool(POOL, Array::default());
                capture.finish_pool();
            }
            capture.finish_registration(Some(0));
            capture.finish_base(true);
            assert_eq!(capture.demand().failure, None);
            capture.lengths()
        };
        assert_eq!((lengths.bases, lengths.registrations), (1, 1));
        assert_eq!(
            (lengths.pools, lengths.countries, lengths.names),
            (count as usize, count as usize, text.len())
        );
        assert_eq!(bases[0].countries, Ok((0, count as usize)));
        assert_eq!(bases[0].host_name, Ok(Some((0, text.len()))));
        assert_eq!(registrations[0].pools, Ok((0, count as usize)));
        assert_eq!(&names[..text.len()], text);
        assert!(
            dense[..count as usize]
                .iter()
                .all(|country| country.country == 9 && country.slot == 2)
        );
        assert!(
            pools[..count as usize]
                .iter()
                .all(|pool| pool.wings_len == 0 && pool.fuel.current == Some(0))
        );
    }
}

#[test]
fn state_pool_projection_preserves_empty_boundaries_without_reading_any_fuel_operand() {
    let memory = Fixture::new();
    let countries = ccountry::Reader::new(&memory, 0, GAME).unwrap();
    let mut resolved = [None; 10];
    for count in [0, 3] {
        let pool = crate::cair_base::state_pool_inputs(
            &memory,
            0,
            memory.indexed()[2].unwrap(),
            POOL,
            Array {
                data: 0xdead0000,
                count,
            },
            17,
            &countries,
            &mut resolved,
        )
        .unwrap();
        assert_eq!((pool.type_index, pool.subunit_index), (i32::MIN, 1));
        assert_eq!((pool.wings_start, pool.wings_len), (17, count));
        assert_eq!(pool.fuel.current, None);
        assert_eq!(pool.fuel.maximum, None);
    }
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|(address, _)| *address >= 0xdead0000)
    );
}
