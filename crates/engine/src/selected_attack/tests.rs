//! Synthetic selected-attack fixtures.
use super::*;
use std::cell::{Cell, RefCell};

struct ObservedMemory {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fail_once: Cell<Option<usize>>,
}
impl ObservedMemory {
    fn new(bytes: usize) -> Self {
        Self {
            bytes: vec![0; bytes],
            reads: RefCell::new(Vec::new()),
            fail_once: Cell::new(None),
        }
    }
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes[address..address + bytes.len()].copy_from_slice(bytes);
    }
    fn attempts(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == address)
            .count()
    }
}
impl Memory for ObservedMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self.fail_once.get() == Some(address) {
            self.fail_once.set(None);
            return false;
        }
        let Some(end) = address.checked_add(destination.len()) else {
            return false;
        };
        let Some(source) = self.bytes.get(address..end) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}

#[test]
fn zero_unit_sample_never_acquires_terrain_domain() {
    let memory = ObservedMemory::new(0);
    let mut packet = UnitsPacket::try_new(UnitCapacity::default()).unwrap();
    let mut geometry = cprovince::GeometryPacket::vacant();
    packet.capture(&memory, 0, 42, &[], Outcome::Rows { len: 0 }, &mut geometry);
    assert_eq!(packet.outcome(), Outcome::Rows { len: 0 });
    assert_eq!(packet.required(), UnitCapacity::default());
    assert!(packet.rows().is_empty() && packet.originals().is_empty());
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn scalar_fault_is_independent_and_shared_stat_alias_is_never_retried() {
    let pointer = 0x1000;
    let mut memory = ObservedMemory::new(0x3000);
    memory.put(pointer, &(stats::VTABLE_RVA as u64).to_le_bytes());
    for (offset, value) in [
        (0x390, -9_i64),
        (0x398, 17),
        (0x368, i64::MIN),
        (0x370, i64::MAX),
        (0x3e0, 23),
        (0x340, -41),
    ] {
        memory.put(pointer + offset, &value.to_le_bytes());
    }
    memory.fail_once.set(Some(pointer + 0x398));
    let mut packet = UnitsPacket::try_new(UnitCapacity {
        rows: 2,
        terrains: 0,
        values: 0,
    })
    .unwrap();
    let first = packet.stat(&memory, 0, Ok(pointer)).unwrap();
    let prior_reads = memory.reads.borrow().len();
    let alias = packet.stat(&memory, 0, Ok(pointer)).unwrap();
    assert_eq!(first, alias);
    assert_eq!(memory.reads.borrow().len(), prior_reads);
    assert_eq!(
        packet.stat_cache[first].scalars,
        [
            Ok(-9),
            Err("selected attack field unreadable"),
            Ok(i64::MIN),
            Ok(i64::MAX),
            Ok(23),
            Ok(-41)
        ]
    );
    assert_eq!(memory.attempts(pointer + 0x398), 1);
}

#[test]
fn malformed_template_identity_is_refused_and_memoized_before_registry_lookup() {
    let wrapper = 0x1000;
    let mut memory = ObservedMemory::new(0x1100);
    memory.put(wrapper, &(template::VTABLE_RVA as u64).to_le_bytes());
    memory.put(
        wrapper + template::TYPE,
        &(template::REFERENCE_TYPE as u64).to_le_bytes(),
    );
    let mut packet = UnitsPacket::try_new(UnitCapacity {
        rows: 2,
        terrains: 0,
        values: 0,
    })
    .unwrap();
    let mut resolver = creference_registry::ScopedResolver::new(0);
    let failure = packet.template(&memory, 0, Ok(wrapper), &mut resolver);
    assert!(matches!(
        failure,
        Err("selected attack template full handle invalid")
    ));
    let attempts = memory.reads.borrow().len();
    assert_eq!(attempts, 2);
    assert!(
        packet
            .template(&memory, 0, Ok(wrapper), &mut resolver)
            .is_err()
    );
    assert_eq!(memory.reads.borrow().len(), attempts);
    let null = packet.template(&memory, 0, Ok(0), &mut resolver).unwrap();
    assert_eq!(null.handle, 0);
    assert!(null.composition().is_err());
    assert_eq!(memory.reads.borrow().len(), attempts);
}

#[test]
fn cached_attack_wraps_with_static_attack_without_reading_token() {
    let entry = 0x1000;
    let mut memory = ObservedMemory::new(0x1100);
    memory.put(entry, &(0x0277_3a70_u64).to_le_bytes());
    memory.put(entry + 8, &0xffff_ffff_u32.to_le_bytes());
    memory.put(entry + 0x10, &i64::MAX.to_le_bytes());
    let value = attack_terrain_from_controls(
        &memory,
        0,
        Array {
            data: entry,
            count: 1,
        },
        TerrainSource {
            pointer: 0xdead,
            index: 1,
            static_attack: Ok(1),
        },
    );
    assert_eq!(value, Ok(i64::MIN));
    assert_eq!(&*memory.reads.borrow(), &[(entry, 8), (entry + 0x10, 8)]);
    let reads = memory.reads.borrow().len();
    assert!(
        attack_terrain_from_controls(
            &memory,
            0,
            Array {
                data: entry,
                count: 1
            },
            TerrainSource {
                pointer: 0,
                index: 0,
                static_attack: Ok(0)
            }
        )
        .is_err()
    );
    assert_eq!(memory.reads.borrow().len(), reads);
}

#[test]
fn raw_root_failure_and_absence_are_memoized_for_the_whole_pass() {
    let root = 0x1000;
    let memory = ObservedMemory::new(0x1100); // Valid empty accumulator.
    let mut metadata = ScalarMetadata::try_new(1).unwrap();
    metadata.set_guard(Ok(()));
    assert_eq!(
        metadata.value(&memory, 0, root, map::LAND_NIGHT_ATTACK),
        Ok(0)
    );
    assert_eq!(
        metadata.value(&memory, 0, root, map::LAND_BUNKER_FACTOR),
        Ok(0)
    );
    assert_eq!(memory.attempts(root), 1);
    metadata.clear();
    metadata.set_guard(Ok(()));
    memory.fail_once.set(Some(root));
    let failed = metadata.value(&memory, 0, root, map::LAND_NIGHT_ATTACK);
    assert!(failed.is_err());
    assert_eq!(
        metadata.value(&memory, 0, root, map::LAND_BUNKER_FACTOR),
        failed
    );
    assert_eq!(memory.attempts(root), 2);
    metadata.clear();
    metadata.set_guard(Ok(()));
    assert_eq!(
        metadata.value(&memory, 0, root, map::LAND_NIGHT_ATTACK),
        Ok(0)
    );
    assert_eq!(memory.attempts(root), 3);
}

#[test]
fn weather_packets_keep_exact_hour_and_independent_refusals() {
    let pointer = 0x1000;
    let memory = ObservedMemory::new(0x1100);
    let mut metadata = ScalarMetadata::try_new(1).unwrap();
    metadata.set_guard(Ok(()));
    let value = WeatherAttack::capture_authenticated(&memory, 0, pointer, -12, &mut metadata);
    assert_eq!(
        (value.sample_hour, value.attack, value.breakthrough),
        (-12, Ok(0), Ok(0))
    );
    let refusal = WeatherAttack::refused(-13, "independent raw-copy admission failure");
    assert_eq!(refusal.sample_hour, -13);
    assert_eq!(
        refusal.attack,
        Err("independent raw-copy admission failure")
    );
    assert_eq!(refusal.breakthrough, refusal.attack);
}

#[test]
fn original_borrows_retire_while_owned_scalar_spans_survive() {
    let mut packet = UnitsPacket::try_new(UnitCapacity {
        rows: 1,
        terrains: 1,
        values: 1,
    })
    .unwrap();
    packet.originals.push(OriginalObservation {
        division: 7,
        pointer: 0x1111,
        stat: Ok(StatObservation { pointer: 0x2222 }),
        template: Ok(TemplateObservation {
            wrapper: 0x3333,
            handle: 52,
        }),
    });
    packet.domain.push(TerrainSource {
        pointer: 0x4444,
        index: 1,
        static_attack: Ok(-31),
    });
    packet.values.push(TerrainValue {
        terrain: 1,
        attack: Ok(-47),
    });
    assert_eq!(
        packet.originals()[0].template.unwrap().composition(),
        Ok(0x3333 + template::COMPOSITION)
    );
    assert!(
        TemplateObservation {
            wrapper: 0,
            handle: 0
        }
        .composition()
        .is_err()
    );
    packet.finish_native();
    assert!(packet.originals().is_empty() && packet.domain.is_empty());
    assert_eq!(
        packet.values(Span { start: 0, len: 1 }),
        Some(
            &[TerrainValue {
                terrain: 1,
                attack: Ok(-47)
            }][..]
        )
    );
    packet.clear();
    assert!(packet.values(Span { start: 0, len: 1 }).is_none());
}

// Selected-attack fixtures reuse `creference_registry::tests::Fixture`
// and `ccountry::synthetic::World`.

struct CaptureFixture {
    source: creference_registry::tests::Fixture,
    reads: RefCell<Vec<(usize, usize)>>,
    fault_once: Cell<Option<usize>>,
}
impl CaptureFixture {
    fn new(source: creference_registry::tests::Fixture) -> Self {
        Self {
            source,
            reads: RefCell::new(Vec::new()),
            fault_once: Cell::new(None),
        }
    }
    fn put(&mut self, address: usize, value: &[u8]) {
        self.source.put(address, value);
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + crate::cpdx_array::CAPACITY, &count.to_le_bytes());
        self.put(address + crate::cpdx_array::COUNT, &count.to_le_bytes());
    }
    fn accumulator(&mut self, root: usize, data: usize, entries: &[(u32, i64)]) {
        self.array(root, data, entries.len() as u32);
        for (index, &(key, raw)) in entries.iter().enumerate() {
            let address = data + index * crate::modifier_entry::STRIDE;
            self.put(address, &[0; crate::modifier_entry::STRIDE]);
            self.put(address + crate::modifier_entry::KEY, &key.to_le_bytes());
            self.put(address + crate::modifier_entry::VALUE, &raw.to_le_bytes());
        }
    }
    fn metadata(&mut self, key: u32, flags: u32) {
        use crate::modifier_definition as definition;
        const TABLE: usize = 0x40_0000;
        self.put(definition::TABLE_RVA, &(TABLE as u64).to_le_bytes());
        let row = TABLE + key as usize * definition::STRIDE;
        self.put(row + definition::FEATURE_SELECTOR, &0_u32.to_le_bytes());
        self.put(row + definition::FLAGS, &flags.to_le_bytes());
    }
    fn attempts(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == address)
            .count()
    }
}
impl Memory for CaptureFixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self.fault_once.get().is_some_and(|fault| {
            (address..address.saturating_add(destination.len())).contains(&fault)
        }) {
            self.fault_once.set(None);
            destination.fill(0xab);
            return false;
        }
        self.source.copy(address, destination)
    }
}

fn class_zero_seed(division: u64, pointer: usize) -> carmy::DivisionSeed {
    let mut acclimatization = [0; 0x40];
    acclimatization[..8]
        .copy_from_slice(&(crate::acclimatization_data::VTABLE_RVA as u64).to_le_bytes());
    carmy::DivisionSeed {
        division,
        pointer,
        handle: Some(division),
        class: Some(0),
        primary_vtable: Some(carmy::VTABLE_RVA as u64),
        secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA as u64),
        roster_owner: Some(0),
        expeditionary_owner: Some(0),
        orders_group: Some(0),
        acclimatization: Some(acclimatization),
        ..Default::default()
    }
}

// Packet-specific sparse addresses leave the existing registry's table/buckets
// intact. Terrain 7 is referenced before terrain 1; the final domain is sorted.
struct PopulatedUnitsFixture {
    memory: CaptureFixture,
    seeds: [carmy::DivisionSeed; 2],
    wrapper: usize,
    template_handle: u64,
    stat: usize,
    adjusters: usize,
}
fn populated_units_fixture() -> PopulatedUnitsFixture {
    use crate::{cgame_state, cterrain_type as terrain, province_definition as definition};
    const GAME_STATE: usize = 0x4000;
    const CENSUS: usize = 0x5000;
    const STAT: usize = 0xc000;
    const WRAPPER: usize = 0xd000;
    const ADJUSTERS: usize = 0xe000;
    let template_handle = (23_u64 << 32) | u64::from(template::REFERENCE_TYPE);
    let mut registry = creference_registry::tests::Fixture::new(template::REFERENCE_TYPE);
    registry.entry(
        creference_registry::tests::Fixture::index(template_handle),
        1,
        template_handle,
        WRAPPER as u64,
    );
    // The reused helper owns its registry header; obtain its synthetic table
    // address rather than introducing a second registry layout implementation.
    let root = creference_registry::LOW_ROOTS_RVA + template::REFERENCE_TYPE as usize * 8;
    let table = crate::read::<u64>(&registry, root).unwrap() as usize;
    registry.put(table + creference_registry::COUNT, &1_u32.to_le_bytes());
    let mut memory = CaptureFixture::new(registry);
    memory.array(GAME_STATE + cgame_state::PROVINCES, CENSUS, 4);
    memory.put(CENSUS, &0_u64.to_le_bytes());
    for (id, object, def, terrain_object) in [
        (1_u32, 0x6000_usize, 0x8000_usize, 0xa000_usize),
        (2, 0x6400, 0x8400, 0xa400),
        (3, 0x6800, 0x8800, 0xa000),
    ] {
        memory.put(CENSUS + id as usize * 8, &(object as u64).to_le_bytes());
        memory.put(object, &(cprovince::VTABLE_RVA as u64).to_le_bytes());
        memory.put(object + cprovince::ID, &id.to_le_bytes());
        memory.put(object + cprovince::DEFINITION, &(def as u64).to_le_bytes());
        memory.put(def + definition::ID, &id.to_le_bytes());
        memory.put(
            def + definition::TERRAIN,
            &(terrain_object as u64).to_le_bytes(),
        );
    }
    for (object, index, static_attack) in [(0xa000_usize, 7_i32, -31_i64), (0xa400, 1, 1)] {
        memory.put(object, &(terrain::VTABLE_RVA as u64).to_le_bytes());
        memory.put(object + terrain::DATABASE_INDEX, &index.to_le_bytes());
        memory.put(object + 0x168, &0x0277_3a70_u64.to_le_bytes());
        memory.put(object + 0x178, &static_attack.to_le_bytes());
    }
    memory.put(STAT, &(stats::VTABLE_RVA as u64).to_le_bytes());
    for (offset, raw) in [
        (0x390, -9_i64),
        (0x398, 17),
        (0x368, i64::MIN),
        (0x370, i64::MAX),
        (0x3e0, 23),
        (0x340, -41),
    ] {
        memory.put(STAT + offset, &raw.to_le_bytes());
    }
    memory.array(STAT + stats::MOVEMENT_TERRAIN_ADJUSTERS, ADJUSTERS, 7);
    for (index, raw) in [(0_usize, i64::MAX), (6, -47)] {
        let entry = ADJUSTERS + index * 0x28;
        memory.put(entry, &0x0277_3a70_u64.to_le_bytes());
        memory.put(entry + 0x10, &raw.to_le_bytes());
        // No token bytes are supplied: a numerical attack read must not need them.
    }
    memory.put(WRAPPER, &(template::VTABLE_RVA as u64).to_le_bytes());
    memory.put(WRAPPER + template::TYPE, &template_handle.to_le_bytes());
    let seeds = [(9_u64 << 32) | 51, (7_u64 << 32) | 51].map(|division| carmy::DivisionSeed {
        stat_block: Some(STAT),
        standard_template: Some(WRAPPER),
        ..class_zero_seed(division, 0xf000 + (division >> 32) as usize * 0x100)
    });
    PopulatedUnitsFixture {
        memory,
        seeds,
        wrapper: WRAPPER,
        template_handle,
        stat: STAT,
        adjusters: ADJUSTERS,
    }
}

#[test]
fn populated_units_capture_authenticates_templates_and_deduplicates_stat_terrain() {
    let fixture = populated_units_fixture();
    let mut geometry =
        cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
            provinces: 4,
            neighbour_entries: 0,
        })
        .unwrap();
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    let mut packet = UnitsPacket::try_new(UnitCapacity {
        rows: 2,
        terrains: 4,
        values: 2,
    })
    .unwrap();
    let allocated = packet.allocated_bytes();
    packet.capture(
        &fixture.memory,
        0,
        -17,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &mut geometry,
    );
    assert_eq!(packet.outcome(), Outcome::Rows { len: 2 });
    assert_eq!(
        packet.required(),
        UnitCapacity {
            rows: 2,
            terrains: 4,
            values: 2
        }
    );
    assert_eq!(packet.allocated_bytes(), allocated);
    assert_eq!(
        packet
            .rows()
            .iter()
            .map(|row| row.division)
            .collect::<Vec<_>>(),
        vec![fixture.seeds[1].division, fixture.seeds[0].division]
    );
    for row in packet.rows() {
        assert_eq!(
            (row.sample_hour, row.template, row.acclimatization),
            (-17, Ok(fixture.template_handle), Ok(100_000))
        );
        assert_eq!(
            row.stats,
            [Ok(-9), Ok(17), Ok(i64::MIN), Ok(i64::MAX), Ok(23), Ok(-41)]
        );
        assert_eq!(
            packet.values(row.terrain.unwrap()).unwrap(),
            &[
                TerrainValue {
                    terrain: 1,
                    attack: Ok(i64::MIN)
                },
                TerrainValue {
                    terrain: 7,
                    attack: Ok(-78)
                }
            ]
        );
    }
    assert_eq!(packet.rows()[0].terrain, packet.rows()[1].terrain);
    for original in packet.originals() {
        assert_eq!(original.stat.unwrap().pointer, fixture.stat);
        let template = original.template.unwrap();
        assert_eq!(
            (template.wrapper, template.handle),
            (fixture.wrapper, fixture.template_handle)
        );
        assert_eq!(
            template.composition(),
            Ok(fixture.wrapper + template::COMPOSITION)
        );
    }
    assert_eq!(fixture.memory.attempts(fixture.stat), 1);
    assert_eq!(fixture.memory.attempts(fixture.wrapper), 1);
    assert_eq!(fixture.memory.attempts(fixture.adjusters + 0x10), 1);
    assert_eq!(
        fixture.memory.attempts(fixture.adjusters + 6 * 0x28 + 0x10),
        1
    );
    assert_eq!(fixture.memory.attempts(fixture.adjusters + 8), 0);
    let spans: Vec<_> = packet
        .rows()
        .iter()
        .map(|row| row.terrain.unwrap())
        .collect();
    let reads = fixture.memory.reads.borrow().len();
    packet.finish_native();
    assert!(packet.originals().is_empty());
    assert!(packet.values(spans[0]).is_some());
    assert_eq!(fixture.memory.reads.borrow().len(), reads);
}

#[test]
fn populated_units_capture_preserves_one_independent_fault_for_both_stat_aliases() {
    let fixture = populated_units_fixture();
    fixture.memory.fault_once.set(Some(fixture.stat + 0x398));
    let mut geometry =
        cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
            provinces: 4,
            neighbour_entries: 0,
        })
        .unwrap();
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    let mut packet = UnitsPacket::try_new(UnitCapacity {
        rows: 2,
        terrains: 4,
        values: 2,
    })
    .unwrap();
    packet.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &mut geometry,
    );
    assert_eq!(packet.outcome(), Outcome::Rows { len: 2 });
    for row in packet.rows() {
        assert_eq!(
            row.stats,
            [
                Ok(-9),
                Err("selected attack field unreadable"),
                Ok(i64::MIN),
                Ok(i64::MAX),
                Ok(23),
                Ok(-41)
            ]
        );
        assert_eq!(row.template, Ok(fixture.template_handle));
        assert_eq!(row.acclimatization, Ok(100_000));
        assert_eq!(
            packet.values(row.terrain.unwrap()).unwrap()[1].attack,
            Ok(-78)
        );
    }
    assert_eq!(fixture.memory.attempts(fixture.stat + 0x398), 1);
    // A fresh hour retires the refusal; the same hour never retries it.
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    packet.capture(
        &fixture.memory,
        0,
        45,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &mut geometry,
    );
    assert!(
        packet
            .rows()
            .iter()
            .all(|row| row.sample_hour == 45 && row.stats[1] == Ok(17))
    );
    assert_eq!(fixture.memory.attempts(fixture.stat + 0x398), 2);
}

#[test]
fn transport_borrows_original_success_and_fault_once_before_native_retirement() {
    use crate::selected_transport_units::{Capacity, Packet, Transfer};
    let mut fixture = populated_units_fixture();
    fixture
        .memory
        .array(fixture.wrapper + template::SUBUNIT_COUNTS, 0x31000, 0);
    fixture
        .memory
        .put(fixture.stat + 0x2b8, &(-123i64).to_le_bytes());
    for seed in &mut fixture.seeds {
        seed.naval_transfer_membership = Some(0);
    }
    let mut geometry =
        cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
            provinces: 4,
            neighbour_entries: 0,
        })
        .unwrap();
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    let mut originals = UnitsPacket::try_new(UnitCapacity {
        rows: 2,
        terrains: 4,
        values: 2,
    })
    .unwrap();
    originals.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &mut geometry,
    );
    fixture.memory.fault_once.set(Some(fixture.stat + 0x2b8));
    let mut transport = Packet::try_new(Capacity {
        rows: 2,
        members: 0,
    })
    .unwrap();
    let allocated = transport.allocated_bytes();
    transport.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &originals,
    );
    assert_eq!(transport.outcome(), Outcome::Rows { len: 2 });
    assert_eq!(transport.allocated_bytes(), allocated);
    for row in transport.rows() {
        assert_eq!(row.sample_hour, 44);
        assert!(row.weight.is_err());
        assert_eq!(row.coastal, Ok(true));
        assert_eq!(row.transfer, Ok(Transfer::Absent));
    }
    assert_eq!(fixture.memory.attempts(fixture.stat), 1);
    assert_eq!(fixture.memory.attempts(fixture.wrapper), 1);
    assert_eq!(fixture.memory.attempts(fixture.stat + 0x2b8), 1);
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.wrapper + template::SUBUNIT_COUNTS),
        1
    );
    assert!(
        transport
            .rows()
            .windows(2)
            .all(|pair| pair[0].unit < pair[1].unit)
    );
    let reads = fixture.memory.reads.borrow().len();
    originals.finish_native();
    assert_eq!(fixture.memory.reads.borrow().len(), reads);
    // Retired certificates cannot be replaced by a fresh read of the selector,
    // stat or template. Transfer null is independently still supplied by seed.
    transport.capture(
        &fixture.memory,
        0,
        45,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &originals,
    );
    assert!(transport.rows().iter().all(|row| row.weight.is_err()
        && row.coastal.is_err()
        && row.sample_hour == 45
        && row.transfer == Ok(Transfer::Absent)));
    assert_eq!(fixture.memory.reads.borrow().len(), reads);
}

#[test]
fn transport_short_member_arena_reports_live_demand_then_growth_and_reset_clear_native_controls() {
    use crate::selected_transport_units::{Capacity, Packet};
    let mut fixture = populated_units_fixture();
    const MEMBERS: usize = 0x31000;
    const MEMBER: usize = 0x32000;
    fixture
        .memory
        .array(fixture.wrapper + template::SUBUNIT_COUNTS, MEMBERS, 1);
    fixture.memory.put(MEMBERS, &(MEMBER as u64).to_le_bytes());
    fixture.memory.put(
        MEMBER,
        &(crate::csub_unit_definition::VTABLE_RVA as u64).to_le_bytes(),
    );
    fixture.memory.put(MEMBER + 0x5c0, &[7]);
    fixture
        .memory
        .put(fixture.stat + 0x2b8, &i64::MAX.to_le_bytes());
    for seed in &mut fixture.seeds {
        seed.naval_transfer_membership = Some(0);
    }
    let mut geometry =
        cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
            provinces: 4,
            neighbour_entries: 0,
        })
        .unwrap();
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    let mut originals = UnitsPacket::try_new(UnitCapacity {
        rows: 2,
        terrains: 4,
        values: 2,
    })
    .unwrap();
    originals.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &mut geometry,
    );
    let mut transport = Packet::try_new(Capacity {
        rows: 2,
        members: 0,
    })
    .unwrap();
    transport.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &originals,
    );
    assert_eq!(
        transport.required(),
        Capacity {
            rows: 2,
            members: 1
        }
    );
    assert!(
        transport
            .rows()
            .iter()
            .all(|row| row.coastal.is_err() && row.weight == Ok(i64::MAX))
    );
    assert_eq!(fixture.memory.attempts(MEMBER), 0);
    transport
        .reserve(Capacity {
            rows: 2,
            members: 1,
        })
        .unwrap();
    // No carried native original may lend the new sample's weight/template.
    transport.capture(
        &fixture.memory,
        0,
        45,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &originals,
    );
    assert!(
        transport
            .rows()
            .iter()
            .all(|row| row.coastal == Err("selected transport original sample is stale"))
    );
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    originals.capture(
        &fixture.memory,
        0,
        45,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &mut geometry,
    );
    transport.capture(
        &fixture.memory,
        0,
        45,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &originals,
    );
    assert!(
        transport
            .rows()
            .iter()
            .all(|row| row.coastal == Ok(true) && row.sample_hour == 45)
    );
    assert_eq!(fixture.memory.attempts(MEMBER), 1);
    transport.clear();
    assert!(transport.rows().is_empty());
    assert_eq!(transport.required(), Capacity::default());
    assert!(matches!(transport.outcome(), Outcome::Unavailable { .. }));
    let reads = fixture.memory.reads.borrow().len();
    transport.capture(
        &fixture.memory,
        0,
        46,
        &fixture.seeds,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1,
        },
        &originals,
    );
    assert!(matches!(transport.outcome(), Outcome::Unavailable { .. }));
    assert!(transport.rows().is_empty());
    assert_eq!(transport.required().rows, 2);
    assert_eq!(fixture.memory.reads.borrow().len(), reads);
}

struct PopulatedMapFixture {
    memory: CaptureFixture,
    seeds: [carmy::DivisionSeed; 2],
    provinces: [cprovince::Row; 3],
    reserved: usize,
    country_values: usize,
    state_values: usize,
    state: usize,
}
fn populated_map_fixture() -> PopulatedMapFixture {
    use crate::{ccountry::synthetic::World, cstate};
    let mut world = World::new(0, 2, 0x900);
    let state = world.alloc(0x900);
    let buckets = world.alloc(2 * cstate::COUNTRY_MODIFIER_STRIDE);
    let reserved = world.object(0);
    let table = crate::read::<u64>(
        &world.memory(),
        world.game_state() + crate::cgame_state::COUNTRIES,
    )
    .unwrap() as usize;
    let links = crate::read::<u64>(
        &world.memory(),
        world.game_state() + crate::cgame_state::COUNTRY_LINKS,
    )
    .unwrap() as usize;
    world.u64(table, reserved as u64);
    world.u64(reserved, ccountry::VTABLE_RVA as u64);
    world.u32(reserved + ccountry::ID, 0);
    world.u32(links + 2 * 4, 0); // Positive raw2 is a readable alias to actual slot0.
    world.u64(state, cstate::VTABLE_RVA as u64);
    world.u64(state + cstate::COUNTRY_MODIFIERS, buckets as u64);
    world.put(buckets + 4, &[1]);
    world.u32(buckets + 8, 0); // Actual controller object's +ID selects this override.
    let mut memory = CaptureFixture::new(creference_registry::tests::Fixture::default());
    memory.put(crate::ccountry::synthetic::START, &world.bytes);
    const COUNTRY_VALUES: usize = 0x20_0000;
    const STATE_VALUES: usize = 0x20_0100;
    const BASE_VALUES: usize = 0x20_0200;
    memory.accumulator(
        reserved + ccountry::MODIFIER_ACCUMULATOR,
        COUNTRY_VALUES,
        &[
            (map::LAND_NIGHT_ATTACK, -9_007_199_254_740_993),
            (map::LAND_BUNKER_FACTOR, 13),
        ],
    );
    memory.accumulator(
        buckets + 0x20,
        STATE_VALUES,
        &[(map::LAND_BUNKER_FACTOR, -47)],
    );
    memory.accumulator(
        state + cstate::BASE_MODIFIERS,
        BASE_VALUES,
        &[(map::LAND_BUNKER_FACTOR, 31)],
    );
    memory.metadata(map::LAND_NIGHT_ATTACK, 0);
    memory.metadata(map::LAND_BUNKER_FACTOR, 0);
    let seeds = [0_u32, 2].map(|raw| carmy::DivisionSeed {
        roster_owner: Some(raw),
        ..class_zero_seed(
            (u64::from(raw) + 1) << 32 | 51,
            0x30_0000 + raw as usize * 0x100,
        )
    });
    let provinces = [
        cprovince::Row {
            id: 1,
            controller: 0,
            attack_fort_sum: Some(9),
            attack_state: Some(0xdead_beef),
            ..Default::default()
        },
        cprovince::Row {
            id: 2,
            controller: 2,
            attack_fort_sum: Some(2),
            attack_state: Some(0),
            ..Default::default()
        },
        cprovince::Row {
            id: 3,
            controller: 2,
            attack_fort_sum: Some(3),
            attack_state: Some(state),
            ..Default::default()
        },
    ];
    PopulatedMapFixture {
        memory,
        seeds,
        provinces,
        reserved,
        country_values: COUNTRY_VALUES,
        state_values: STATE_VALUES,
        state,
    }
}

fn capture_populated_map(packet: &mut MapPacket, fixture: &PopulatedMapFixture, hour: i32) {
    packet.capture(
        &fixture.memory,
        0,
        crate::ccountry::synthetic::START,
        hour,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &fixture.provinces,
        Outcome::Rows { len: 3 },
        Ok(()),
    );
}

#[test]
fn map_capture_distinguishes_literal_controller_zero_positive_alias_zero_and_null_state() {
    let fixture = populated_map_fixture();
    let mut packet = MapPacket::try_new(MapCapacity {
        countries: 3,
        links: 3,
        provinces: 3,
    })
    .unwrap();
    let allocated = packet.allocated_bytes();
    capture_populated_map(&mut packet, &fixture, -19);
    assert_eq!(
        packet.outcomes(),
        [Outcome::Rows { len: 1 }, Outcome::Rows { len: 3 }]
    );
    assert_eq!(
        packet.required(),
        MapCapacity {
            countries: 3,
            links: 3,
            provinces: 3
        }
    );
    assert_eq!(packet.failures(), &[None; 3]);
    assert_eq!(packet.allocated_bytes(), allocated);
    let country = packet.countries()[0];
    assert_eq!(
        (country.country, country.canonical_slot, country.sample_hour),
        (0, 0, -19)
    );
    assert_eq!(country.land_night_attack, Ok(-9_007_199_254_740_993));
    assert_eq!(country.land_bunker_effectiveness_factor, Ok(13));
    let provinces = packet.provinces();
    assert_eq!(provinces[0].fort_sum, Ok(9));
    assert_eq!(
        provinces[0].land_bunker_effectiveness_factor,
        Err("selected attack modifier undemanded")
    );
    assert_eq!(provinces[1].land_bunker_effectiveness_factor, Ok(0));
    assert_eq!(provinces[2].land_bunker_effectiveness_factor, Ok(-47));
    assert!(provinces.iter().all(|row| row.sample_hour == -19));
    assert_eq!(fixture.memory.attempts(0xdead_beef), 0);
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.reserved + ccountry::MODIFIER_ACCUMULATOR),
        1
    );
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.state + crate::cstate::BASE_MODIFIERS),
        0
    );
}

#[test]
fn map_capture_reports_small_arenas_then_uses_checked_growth_and_fresh_reused_storage() {
    let mut fixture = populated_map_fixture();
    let mut packet = MapPacket::vacant();
    capture_populated_map(&mut packet, &fixture, 70);
    assert_eq!(
        packet.outcomes(),
        [Outcome::Unavailable {
            reason: "selected attack country admission arena exhausted"
        }; 2]
    );
    assert_eq!(
        packet.required(),
        MapCapacity {
            countries: 3,
            links: 3,
            provinces: 3
        }
    );
    assert_eq!(fixture.memory.attempts(fixture.reserved), 0);
    assert_eq!(packet.allocated_bytes(), 0);
    packet
        .reserve(MapCapacity {
            countries: 3,
            links: 3,
            provinces: 0,
        })
        .unwrap();
    capture_populated_map(&mut packet, &fixture, 71);
    assert_eq!(
        packet.outcomes(),
        [Outcome::Unavailable {
            reason: "selected attack province suffix arena exhausted"
        }; 2]
    );
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.reserved + ccountry::MODIFIER_ACCUMULATOR),
        0
    );
    packet.reserve(packet.required()).unwrap();
    let allocated = packet.allocated_bytes();
    capture_populated_map(&mut packet, &fixture, 72);
    assert_eq!(
        packet.outcomes(),
        [Outcome::Rows { len: 1 }, Outcome::Rows { len: 3 }]
    );
    assert_eq!(
        packet.countries()[0].land_night_attack,
        Ok(-9_007_199_254_740_993)
    );
    fixture.memory.put(
        fixture.country_values + crate::modifier_entry::VALUE,
        &17_i64.to_le_bytes(),
    );
    fixture.memory.put(
        fixture.state_values + crate::modifier_entry::VALUE,
        &(-29_i64).to_le_bytes(),
    );
    capture_populated_map(&mut packet, &fixture, 73);
    assert_eq!(packet.countries()[0].land_night_attack, Ok(17));
    assert_eq!(
        packet.provinces()[2].land_bunker_effectiveness_factor,
        Ok(-29)
    );
    assert!(packet.countries().iter().all(|row| row.sample_hour == 73));
    assert!(packet.provinces().iter().all(|row| row.sample_hour == 73));
    assert_eq!(packet.allocated_bytes(), allocated);
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.reserved + ccountry::MODIFIER_ACCUMULATOR),
        2
    );
}

fn install_group_fixture(fixture: &mut PopulatedUnitsFixture) -> [crate::corders_group::Root; 3] {
    use crate::{carmy_leader as army, corders_group as group, cunit_leader as unit};
    const LEADER: usize = 0x50000;
    let leader = (31_u64 << 32) | u64::from(unit::REFERENCE_TYPE);
    let registry_root = if unit::REFERENCE_TYPE < creference_registry::LOW_ROOT_COUNT {
        creference_registry::LOW_ROOTS_RVA + unit::REFERENCE_TYPE as usize * 8
    } else if unit::REFERENCE_TYPE < creference_registry::MID_TYPE_LIMIT {
        creference_registry::MID_ROOT_RVA
    } else {
        creference_registry::HIGH_ROOT_RVA
    };
    fixture.memory.put(registry_root, &0x1000_u64.to_le_bytes());
    fixture.memory.source.entry(
        creference_registry::tests::Fixture::index(leader),
        1,
        leader,
        LEADER as u64,
    );
    fixture
        .memory
        .put(LEADER, &(army::VTABLE_RVA as u64).to_le_bytes());
    fixture
        .memory
        .put(LEADER + unit::TYPE, &leader.to_le_bytes());
    fixture.memory.put(LEADER + 0xe84, &0_i32.to_le_bytes());
    fixture
        .memory
        .put(LEADER + army::FORMATION_REFERENCE, &0_u64.to_le_bytes());
    for (offset, raw) in [
        (0xfb8, -17_i64),
        (0xfe0, 23),
        (0x1030, i64::MIN),
        (0x270, i64::MAX),
    ] {
        fixture.memory.put(LEADER + offset, &raw.to_le_bytes());
    }
    fixture
        .memory
        .array(LEADER + army::MOVEMENT_TERRAIN_DATA, 0x72000, 8);
    for (index, value) in [(1_usize, -79_i64), (7, 101)] {
        fixture
            .memory
            .put(0x72000 + index * 0x28 + 0x10, &value.to_le_bytes());
    }
    [0x60000_usize, 0x60400, 0x60800].map(|pointer| {
        let handle = (((pointer / 0x400) as u64) << 32) | u64::from(group::REFERENCE_TYPE);
        fixture
            .memory
            .put(pointer, &(group::VTABLE_RVA as u64).to_le_bytes());
        fixture
            .memory
            .put(pointer + group::HANDLE, &handle.to_le_bytes());
        fixture
            .memory
            .put(pointer + group::COUNTRY, &9_u32.to_le_bytes());
        fixture.memory.put(pointer + group::WITHDRAWING, &[0]);
        let direct = if pointer == 0x60800 { 0 } else { LEADER };
        fixture
            .memory
            .put(pointer + group::LEADER, &(direct as u64).to_le_bytes());
        group::Root {
            pointer,
            handle,
            country: 9,
        }
    })
}

#[test]
fn zero_divisions_capture_all_current_groups_and_share_original_commander_observations() {
    let mut fixture = populated_units_fixture();
    let roots = install_group_fixture(&mut fixture);
    let mut controls = [crate::corders_group::DirectCommander::default(); 3];
    let mut prefix = [crate::corders_group::CounterInputsRow::default(); 3];
    let source = Outcome::Rows { len: 3 };
    assert_eq!(
        crate::corders_group::walk_counter_inputs_with_commanders(
            &fixture.memory,
            0,
            0x4000,
            &roots,
            source,
            &mut prefix,
            &mut controls
        ),
        source
    );
    assert_eq!(prefix.map(|row| row.leader_enroute_days), [Some(0); 3]);
    let mut geometry =
        cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
            provinces: 4,
            neighbour_entries: 0,
        })
        .unwrap();
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    let mut units = UnitsPacket::try_new(UnitCapacity {
        rows: 0,
        terrains: 4,
        values: 0,
    })
    .unwrap();
    units.capture(
        &fixture.memory,
        0,
        91,
        &[],
        Outcome::Rows { len: 0 },
        &mut geometry,
    );
    assert_eq!(units.required().terrains, 0);
    let mut groups = GroupsPacket::try_new(GroupCapacity { rows: 3, values: 2 }).unwrap();
    groups.capture(
        &fixture.memory,
        0,
        91,
        &controls,
        source,
        &mut units,
        &mut geometry,
        Ok(()),
    );
    assert_eq!(groups.outcome(), source);
    assert_eq!(groups.required(), GroupCapacity { rows: 3, values: 2 });
    assert_eq!(units.outcome(), Outcome::Rows { len: 0 });
    assert_eq!(groups.rows()[2].commander, Ok(None));
    assert!(groups.rows().iter().all(|row| row.sample_hour == 91));
    let commander = groups.rows()[0].commander.unwrap().unwrap();
    assert_eq!(
        commander.scalars,
        [Ok(-17), Ok(23), Ok(i64::MIN), Ok(i64::MAX)]
    );
    assert_eq!(
        groups.values(commander.terrain.unwrap()).unwrap(),
        &[
            TerrainValue {
                terrain: 1,
                attack: Ok(-79)
            },
            TerrainValue {
                terrain: 7,
                attack: Ok(101)
            }
        ]
    );
    assert_eq!(groups.rows()[0].commander, groups.rows()[1].commander);
    for root in roots {
        assert_eq!(
            fixture
                .memory
                .attempts(root.pointer + crate::corders_group::LEADER),
            1
        );
    }
    assert_eq!(
        fixture.memory.attempts(0x50000),
        1,
        "shared +88 identity must not be reauthenticated"
    );
    for offset in [0xfb8, 0xfe0, 0x1030, 0x270] {
        assert_eq!(fixture.memory.attempts(0x50000 + offset), 1);
    }
    assert_eq!(fixture.memory.attempts(0xa000), 1);
    assert_eq!(fixture.memory.attempts(0xa400), 1);
    let attempts = fixture.memory.reads.borrow().len();
    groups.finish_native();
    units.finish_native();
    assert_eq!(fixture.memory.reads.borrow().len(), attempts);
    assert!(groups.leaders.is_empty() && units.domain.is_empty());
    assert!(groups.values(commander.terrain.unwrap()).is_some());
}

#[test]
fn independent_group_scalar_fault_and_current_geometry_refusal_preserve_null_and_other_fields() {
    let mut fixture = populated_units_fixture();
    let roots = install_group_fixture(&mut fixture);
    let mut controls = [crate::corders_group::DirectCommander::default(); 3];
    let mut prefix = [crate::corders_group::CounterInputsRow::default(); 3];
    let source = Outcome::Rows { len: 3 };
    crate::corders_group::walk_counter_inputs_with_commanders(
        &fixture.memory,
        0,
        0x4000,
        &roots,
        source,
        &mut prefix,
        &mut controls,
    );
    fixture.memory.fault_once.set(Some(0x50000 + 0xfe0));
    let mut units = UnitsPacket::vacant();
    units.fail("presentation stopped");
    units.refuse_domain_at(-42, "presentation stopped");
    let mut geometry = cprovince::GeometryPacket::vacant();
    let mut groups = GroupsPacket::try_new(GroupCapacity { rows: 3, values: 0 }).unwrap();
    groups.capture(
        &fixture.memory,
        0,
        -42,
        &controls,
        source,
        &mut units,
        &mut geometry,
        Err("presentation stopped"),
    );
    assert_eq!(groups.outcome(), source);
    let commander = groups.rows()[0].commander.unwrap().unwrap();
    assert_eq!(
        commander.scalars,
        [
            Ok(-17),
            Err("selected attack field unreadable"),
            Ok(i64::MIN),
            Ok(i64::MAX)
        ]
    );
    assert_eq!(commander.terrain, Err("presentation stopped"));
    assert_eq!(groups.rows()[1].commander, groups.rows()[0].commander);
    assert_eq!(groups.rows()[2].commander, Ok(None));
    assert!(groups.rows().iter().all(|row| row.sample_hour == -42));
    assert_eq!(fixture.memory.attempts(0x50000 + 0xfe0), 1);
    assert_eq!(
        fixture.memory.attempts(0xa000),
        0,
        "failed current geometry cannot be retried"
    );
}

#[test]
fn null_commander_groups_never_demand_domain_and_selecting_failures_are_not_retried() {
    let mut fixture = populated_units_fixture();
    let roots = install_group_fixture(&mut fixture);
    fixture
        .memory
        .fault_once
        .set(Some(roots[0].pointer + crate::corders_group::LEADER));
    let mut controls = [crate::corders_group::DirectCommander::default(); 3];
    let mut prefix = [crate::corders_group::CounterInputsRow::default(); 3];
    let source = Outcome::Rows { len: 3 };
    crate::corders_group::walk_counter_inputs_with_commanders(
        &fixture.memory,
        0,
        0x4000,
        &roots,
        source,
        &mut prefix,
        &mut controls,
    );
    assert!(controls[0].pointer.is_err() && controls[0].leader.is_err());
    let mut groups = GroupsPacket::try_new(GroupCapacity { rows: 3, values: 0 }).unwrap();
    let mut units = UnitsPacket::vacant();
    let mut geometry = cprovince::GeometryPacket::vacant();
    groups.capture(
        &fixture.memory,
        0,
        52,
        &controls[..1],
        Outcome::Rows { len: 1 },
        &mut units,
        &mut geometry,
        Err("geometry unavailable"),
    );
    assert!(groups.rows()[0].commander.is_err());
    groups.capture(
        &fixture.memory,
        0,
        52,
        &controls[2..],
        Outcome::Rows { len: 1 },
        &mut units,
        &mut geometry,
        Err("geometry unavailable"),
    );
    assert_eq!(groups.rows()[0].commander, Ok(None));
    assert_eq!(units.required().terrains, 0);
    assert_eq!(
        fixture
            .memory
            .attempts(roots[0].pointer + crate::corders_group::LEADER),
        1
    );
    assert_eq!(fixture.memory.attempts(0xa000), 0);
}

#[test]
fn partial_units_sort_every_exit_and_preserve_unobserved_acclimatization_distinction() {
    let fixture = populated_units_fixture();
    let mut seeds = fixture.seeds.to_vec();
    let mut invalid = class_zero_seed((11_u64 << 32) | 51, 0xff00);
    invalid.class = Some(1);
    seeds.push(invalid);
    let mut geometry =
        cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
            provinces: 4,
            neighbour_entries: 0,
        })
        .unwrap();
    geometry.begin(&fixture.memory, 0, 0x4000).unwrap();
    let mut packet = UnitsPacket::try_new(UnitCapacity {
        rows: 3,
        terrains: 4,
        values: 2,
    })
    .unwrap();
    packet.capture(
        &fixture.memory,
        0,
        72,
        &seeds,
        Outcome::Rows { len: 3 },
        &mut geometry,
    );
    assert!(matches!(packet.outcome(), Outcome::Unavailable { .. }));
    assert!(
        packet
            .rows()
            .windows(2)
            .all(|pair| pair[0].division < pair[1].division)
    );
    assert!(
        packet
            .originals()
            .windows(2)
            .all(|pair| pair[0].division < pair[1].division)
    );
    let reads = fixture.memory.reads.borrow().len();
    for seed in &seeds[..2] {
        assert_eq!(packet.acclimatization(seed.division), Some(Ok(100_000)));
        assert!(packet.original(seed.division, seed.pointer).is_some());
    }
    assert_eq!(packet.acclimatization(invalid.division), None);
    assert_eq!(fixture.memory.reads.borrow().len(), reads);
}

#[test]
fn unverified_modifier_guard_refuses_each_scalar_without_native_metadata_reads() {
    let memory = ObservedMemory::new(0x1100);
    let mut metadata = ScalarMetadata::try_new(1).unwrap();
    assert!(
        metadata
            .value(&memory, 0, 0x1000, map::LAND_NIGHT_ATTACK)
            .is_err()
    );
    assert!(memory.reads.borrow().is_empty());
    metadata.set_guard(Err("synthetic changed built-in layout"));
    for key in [
        map::LAND_NIGHT_ATTACK,
        map::LAND_BUNKER_FACTOR,
        map::WEATHER_ATTACK,
        map::WEATHER_BREAKTHROUGH,
    ] {
        assert_eq!(
            metadata.value(&memory, 0, 0x1000, key),
            Err("synthetic changed built-in layout")
        );
    }
    assert!(memory.reads.borrow().is_empty());
    metadata.finish_native();
    assert_eq!(metadata.required(), 0);
}

#[test]
fn refused_map_layout_keeps_current_fort_sums_and_native_domain_outcomes() {
    let fixture = populated_map_fixture();
    let mut packet = MapPacket::try_new(MapCapacity {
        countries: 3,
        links: 3,
        provinces: 3,
    })
    .unwrap();
    packet.capture(
        &fixture.memory,
        0,
        crate::ccountry::synthetic::START,
        103,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &fixture.provinces,
        Outcome::Rows { len: 3 },
        Err("synthetic changed built-in layout"),
    );
    assert_eq!(
        packet.outcomes(),
        [Outcome::Rows { len: 1 }, Outcome::Rows { len: 3 }]
    );
    assert_eq!(
        packet.countries()[0].land_night_attack,
        Err("synthetic changed built-in layout")
    );
    assert_eq!(
        packet.countries()[0].land_bunker_effectiveness_factor,
        Err("synthetic changed built-in layout")
    );
    assert_eq!(
        packet
            .provinces()
            .iter()
            .map(|row| row.fort_sum)
            .collect::<Vec<_>>(),
        fixture
            .provinces
            .iter()
            .map(|row| row.attack_fort_sum.ok_or(UNSUPPLIED))
            .collect::<Vec<_>>()
    );
    assert!(packet.provinces().iter().all(|row| row.sample_hour == 103));
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.reserved + ccountry::MODIFIER_ACCUMULATOR),
        0
    );
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.state_values + crate::modifier_entry::VALUE),
        0
    );
}

#[test]
fn worker_attack_rows_and_group_rows_grow_before_their_first_publication() {
    let mut memory = ObservedMemory::new(0x5000);
    memory.put(
        0x1000 + crate::cgame_state::PROVINCES,
        &0x2000_u64.to_le_bytes(),
    );
    memory.put(
        0x1000 + crate::cgame_state::PROVINCES + 8,
        &1_u32.to_le_bytes(),
    );
    memory.put(
        0x1000 + crate::cgame_state::PROVINCES + 12,
        &1_u32.to_le_bytes(),
    );
    memory.put(0x3000, &(stats::VTABLE_RVA as u64).to_le_bytes());
    let mut geometry = cprovince::GeometryPacket::vacant();
    geometry.enable_growth();
    let mut units = UnitsPacket::vacant();
    units.enable_growth();
    let mut groups = GroupsPacket::vacant();
    groups.enable_growth();
    for count in [5000, 11003] {
        geometry.begin(&memory, 0, 0x1000).unwrap();
        let seeds = (0..count)
            .map(|index| carmy::DivisionSeed {
                division: ((index as u64 + 1) << 32) | 4,
                pointer: 0x4000 + index,
                handle: Some(((index as u64 + 1) << 32) | 4),
                primary_vtable: Some(carmy::VTABLE_RVA as u64),
                secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA as u64),
                class: Some(0),
                stat_block: Some(0x3000),
                standard_template: Some(0),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        units.capture(
            &memory,
            0,
            7,
            &seeds,
            Outcome::Rows { len: count },
            &mut geometry,
        );
        assert_eq!(units.outcome(), Outcome::Rows { len: count });
        assert_eq!(units.rows().len(), count);
        assert_eq!(units.originals().len(), count);
        let controls = (0..count)
            .map(|index| crate::corders_group::DirectCommander {
                group: ((index as u64 + 1) << 32) | crate::corders_group::REFERENCE_TYPE as u64,
                leader: Ok(None),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        groups.capture(
            &memory,
            0,
            7,
            &controls,
            Outcome::Rows { len: count },
            &mut units,
            &mut geometry,
            Ok(()),
        );
        assert_eq!(groups.outcome(), Outcome::Rows { len: count });
    }
}

#[test]
fn worker_stat_terrain_values_and_scalar_roots_keep_large_original_domains() {
    let memory = ObservedMemory::new(0x2000);
    let mut units = UnitsPacket::vacant();
    units.enable_growth();
    units.domain_result = Ok(());
    units.stat_cache.push(StatCache {
        observation: Ok(StatObservation { pointer: 0x1000 }),
        scalars: [Ok(0); 6],
        header: Ok(Array {
            data: 0x1000,
            count: 0,
        }),
        terrain: None,
    });
    units.domain = (1..=11003)
        .map(|index| TerrainSource {
            pointer: index as usize,
            index,
            static_attack: Ok(0),
        })
        .collect();
    let span = units.stat_terrain(&memory, 0, 0).unwrap();
    assert_eq!(span.len, 11003);
    assert_eq!(units.values.len(), 11003);
    let mut roots = ScalarMetadata::default();
    roots.enable_growth();
    roots.set_guard(Ok(()));
    for index in 0..5000 {
        let pointer = 0x10_0000 + index * 0x100;
        let first = roots.value(&memory, 0, pointer, map::LAND_NIGHT_ATTACK);
        let reads = memory.reads.borrow().len();
        assert_eq!(
            roots.value(&memory, 0, pointer, map::LAND_NIGHT_ATTACK),
            first
        );
        assert_eq!(memory.reads.borrow().len(), reads);
    }
    assert_eq!(roots.required(), 5000);
}

#[test]
fn worker_attack_map_grows_five_thousand_countries_and_later_mod_province_population() {
    let world = ccountry::synthetic::World::new(0, 5000, 0x900);
    let memory = world.memory();
    let seeds = (1..=5000)
        .map(|raw| carmy::DivisionSeed {
            roster_owner: Some(raw),
            ..class_zero_seed(((raw as u64) << 32) | 51, 0x10_0000 + raw as usize * 0x100)
        })
        .collect::<Vec<_>>();
    let mut packet = MapPacket::vacant();
    packet.enable_growth();
    for count in [5000, 11003] {
        let provinces = (0..count)
            .map(|index| cprovince::Row {
                id: index as u32 + 1,
                controller: (index % 5000 + 1) as u32,
                attack_fort_sum: Some(1),
                attack_state: Some(0),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        packet.capture(
            &memory,
            0,
            world.game_state(),
            7,
            &seeds,
            Outcome::Rows { len: seeds.len() },
            &provinces,
            Outcome::Rows {
                len: provinces.len(),
            },
            Ok(()),
        );
        assert_eq!(
            packet.outcomes(),
            [Outcome::Rows { len: 5000 }, Outcome::Rows { len: count }]
        );
        assert_eq!(packet.countries().len(), 5000);
        assert_eq!(packet.provinces().len(), count);
        assert!(
            packet
                .countries()
                .iter()
                .all(|row| row.land_night_attack == Ok(0)
                    && row.land_bunker_effectiveness_factor == Ok(0))
        );
        assert!(
            packet
                .provinces()
                .iter()
                .all(|row| row.land_bunker_effectiveness_factor == Ok(0))
        );
    }
}
