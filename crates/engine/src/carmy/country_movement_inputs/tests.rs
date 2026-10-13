use super::*;
use crate::{
    ccountry::synthetic::World, cdiplomacy_status, cgame_state, cmodifier, cpdx_array::Array,
    crelation_status, cweather_manager, modifier_definition,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

const MODULE: usize = 0x4000_0000;

struct Fixture {
    world: World,
    globals: BTreeMap<usize, Vec<u8>>,
    regions: Vec<TransportRegion>,
}

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some((&start, bytes)) = self.globals.range(..=address).next_back()
            && let Some(bytes) = bytes.get(address - start..address - start + output.len())
        {
            output.copy_from_slice(bytes);
            return true;
        }
        self.world.memory().copy(address, output)
    }
}

impl Fixture {
    fn new() -> Self {
        Self::with_countries(3)
    }

    fn with_countries(countries: u32) -> Self {
        let mut world = World::new(MODULE, countries, 0x1100);
        let state = world.game_state();
        let owner = world.object(1);
        let target = world.object(2);
        let memory = world.memory();
        let [countries, links] = ccountry::Reader::new(&memory, MODULE, state)
            .unwrap()
            .arrays();
        // Raw 3 names canonical slot 2, whose current native id is that alias.
        world.u64(countries.element(3, 8).unwrap(), target as u64);
        world.u32(links.element(3, 4).unwrap(), 2);
        world.u32(target + ccountry::ID, 3);

        // Reserve the diplomacy owner backlink before allocating the matrix.
        let diplomacy = world.alloc(cdiplomacy_status::COUNTRY + std::mem::size_of::<u64>());
        let matrix = world.alloc(4 * 8);
        world.u64(owner + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        world.u64(diplomacy, (MODULE + cdiplomacy_status::VTABLE_RVA) as u64);
        world.u64(diplomacy + cdiplomacy_status::COUNTRY, owner as u64);
        world.array(diplomacy + cdiplomacy_status::RELATIONS, matrix, 4);
        for (slot, raw, value) in [
            (0, 0, Some(150_001_i64)),
            (1, 1, None),
            (2, 3, Some(-5_000)),
        ] {
            let status = world.alloc(0x320);
            world.u64(matrix + slot * 8, status as u64);
            world.u64(status, (MODULE + crelation_status::VTABLE_RVA) as u64);
            world.u64(
                status + crelation_status::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
            world.u32(status + crelation_status::TARGET_COUNTRY, raw);
            let accumulator = status + crelation_status::MODIFIER + cmodifier::ACCUMULATOR;
            if let Some(value) = value {
                let entry = world.alloc(16);
                world.array(accumulator, entry, 1);
                world.u32(entry, crate::modifier_keys::RIVER_CROSSING_FACTOR_AGAINST);
                world.u64(entry + 8, value as u64);
            } else {
                world.array(accumulator, 0, 0);
            }
        }
        let metadata = world.alloc(
            (crate::modifier_keys::RIVER_CROSSING_FACTOR_AGAINST as usize + 1)
                * modifier_definition::STRIDE,
        );
        world.u32(
            metadata
                + crate::modifier_keys::RIVER_CROSSING_FACTOR_AGAINST as usize
                    * modifier_definition::STRIDE
                + modifier_definition::FLAGS,
            modifier_definition::CLAMP_MAXIMUM,
        );

        let region_array = world.alloc(4 * 8);
        world.array(state + cgame_state::STRATEGIC_REGIONS, region_array, 4);
        let mut regions = Vec::new();
        for (slot, id) in [(1, 7), (2, 29), (3, 86)] {
            let pointer = world.alloc(0x180);
            world.u64(pointer, (MODULE + cstrategic_region::VTABLE_RVA) as u64);
            world.u32(pointer + cstrategic_region::ID, id);
            world.u32(pointer + cstrategic_region::RUNTIME_SLOT, slot);
            world.u64(region_array + slot as usize * 8, pointer as u64);
            regions.push(TransportRegion { id, pointer });
        }
        let weather = world.alloc(0x100);
        world.u64(state + cgame_state::WEATHER_MANAGER, weather as u64);
        world.u64(weather, (MODULE + cweather_manager::VTABLE_RVA) as u64);
        // The weather query returns no regional weather outside this valid empty array.
        world.array(weather + cweather_manager::REGIONS, 0, 0);
        let globals = BTreeMap::from([
            (
                MODULE + modifier_definition::TABLE_RVA,
                (metadata as u64).to_le_bytes().to_vec(),
            ),
            (
                MODULE + modifier_definition::TRADE_FEATURE_FLAGS_RVA,
                0_u32.to_le_bytes().to_vec(),
            ),
            (
                MODULE + cnaval_unit_transfer::BASE_SPEED_RVA,
                200_000_i64.to_le_bytes().to_vec(),
            ),
        ]);
        Self {
            world,
            globals,
            regions,
        }
    }

    fn owner(&self) -> ccountry::Country {
        ccountry::Reader::new(self, MODULE, self.world.game_state())
            .unwrap()
            .resolve(1)
            .unwrap()
    }
}

struct Observed<'a> {
    source: &'a Fixture,
    fault: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for Observed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self
            .fault
            .is_some_and(|fault| address <= fault && fault < address + output.len())
        {
            return false;
        }
        self.source.copy(address, output)
    }
}

fn observed(source: &Fixture, fault: Option<usize>) -> Observed<'_> {
    Observed {
        source,
        fault,
        reads: RefCell::new(Vec::new()),
    }
}

struct OneShot<'a> {
    source: &'a Fixture,
    fault: usize,
    failed: Cell<bool>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for OneShot<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if !self.failed.get() && address <= self.fault && self.fault < address + output.len() {
            self.failed.set(true);
            return false;
        }
        self.source.copy(address, output)
    }
}

#[test]
fn transient_transport_root_failure_is_sticky_across_owners_until_clear() {
    let source = Fixture::new();
    let memory = OneShot {
        source: &source,
        fault: MODULE + cnaval_unit_transfer::BASE_SPEED_RVA,
        failed: Cell::new(false),
        reads: RefCell::new(Vec::new()),
    };
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let mut output = [RegionTransportSpeed::default(); 3];
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        source.owner(),
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!((capture.len, capture.required), (Ok(3), 3));
    assert_eq!(capture.first_failure, packet.transport_failure());
    assert!(capture.first_failure.is_some());
    assert_eq!(output.map(|row| row.speed), [None; 3]);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(address, _)| *address == memory.fault)
            .count(),
        1
    );
    let other_owner = ccountry::Reader::new(&source, MODULE, source.world.game_state())
        .unwrap()
        .resolve(2)
        .unwrap();
    memory.reads.borrow_mut().clear();
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        other_owner,
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!(capture.len, Ok(3));
    assert_eq!(output.map(|row| row.speed), [None; 3]);
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|(address, _)| *address == memory.fault)
    );
    assert_eq!(
        output.map(|row| row.failure),
        [Some("transport input unreadable"); 3]
    );
    packet.clear();
    assert_eq!(packet.transport_failure(), None);
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        other_owner,
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!(capture.first_failure, None);
    assert_eq!(output.map(|row| row.speed), [Some(200_000); 3]);
    packet.clear();
}

#[test]
fn entered_transport_fault_preserves_healthy_later_regions_and_owners() {
    let mut source = Fixture::new();
    let bad_region = source.regions[1].pointer;
    source
        .world
        .u32(bad_region + cstrategic_region::RUNTIME_SLOT, 4);
    let memory = observed(&source, None);
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let mut output = [RegionTransportSpeed::default(); 3];
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        source.owner(),
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!(capture.len, Ok(3));
    assert_eq!(
        output.map(|row| row.speed),
        [Some(200_000), None, Some(200_000)]
    );
    assert_eq!(
        output.map(|row| row.failure),
        [None, Some("transport region slot outside census"), None]
    );
    assert_eq!(capture.first_failure, packet.transport_failure());
    assert!(memory.reads.borrow().iter().any(|(address, _)| {
        *address == source.regions[2].pointer
            || *address == source.regions[2].pointer + cstrategic_region::ID
    }));
    let attempts = memory
        .reads
        .borrow()
        .iter()
        .filter(|(address, _)| *address == bad_region + cstrategic_region::RUNTIME_SLOT)
        .count();
    let other_owner = ccountry::Reader::new(&source, MODULE, source.world.game_state())
        .unwrap()
        .resolve(2)
        .unwrap();
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        other_owner,
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!(
        capture.first_failure,
        Some("transport region slot outside census")
    );
    assert_eq!(
        output.map(|row| row.speed),
        [Some(200_000), None, Some(200_000)]
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(address, _)| *address == bad_region + cstrategic_region::RUNTIME_SLOT)
            .count(),
        attempts
    );
    packet.clear();
}

#[test]
fn owner_transport_fault_preserves_other_owners_and_its_named_rows_without_retry() {
    let source = Fixture::new();
    let owner = source.owner();
    let memory = OneShot {
        source: &source,
        fault: owner.pointer + ccountry::ID,
        failed: Cell::new(false),
        reads: RefCell::new(Vec::new()),
    };
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let mut output = [RegionTransportSpeed::default(); 3];
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        owner,
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!((capture.len, capture.required), (Ok(3), 3));
    assert_eq!(capture.first_failure, Some("country id unreadable"));
    assert_eq!(output.map(|row| row.speed), [None; 3]);
    assert_eq!(
        output.map(|row| row.failure),
        [Some("country id unreadable"); 3]
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(address, _)| *address == memory.fault)
            .count(),
        1
    );
    let other_owner = ccountry::Reader::new(&source, MODULE, source.world.game_state())
        .unwrap()
        .resolve(2)
        .unwrap();
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        other_owner,
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!(capture.first_failure, None);
    assert_eq!(output.map(|row| row.speed), [Some(200_000); 3]);
    assert_eq!(output.map(|row| row.failure), [None; 3]);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(address, _)| *address == memory.fault)
            .count(),
        1
    );
}

#[test]
fn distinct_transport_faults_do_not_consume_later_healthy_query_capacity() {
    let mut source = Fixture::new();
    for (index, slot) in [(0, 4), (1, 5)] {
        source.world.u32(
            source.regions[index].pointer + cstrategic_region::RUNTIME_SLOT,
            slot,
        );
    }
    let memory = observed(&source, None);
    let mut packet = CountryMovementPacket::try_new(1).unwrap();
    packet.reserve_transport(6, source.regions.len()).unwrap();
    let mut output = [RegionTransportSpeed::default(); 3];
    for raw in [1, 2] {
        let owner = ccountry::Reader::new(&source, MODULE, source.world.game_state())
            .unwrap()
            .resolve(raw)
            .unwrap();
        let capture = capture_transport(
            &memory,
            MODULE,
            source.world.game_state(),
            owner,
            &source.regions,
            &mut output,
            &mut packet,
        );
        assert_eq!(capture.len, Ok(3));
        assert_eq!(output.map(|row| row.speed), [None, None, Some(200_000)]);
        assert_eq!(
            output.map(|row| row.failure),
            [
                Some("transport region slot outside census"),
                Some("transport region slot outside census"),
                None
            ]
        );
    }
    for region in &source.regions[..2] {
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|(address, _)| *address == region.pointer + cstrategic_region::RUNTIME_SLOT)
                .count(),
            1
        );
    }
}

#[test]
fn transport_refusal_capacity_tracks_five_thousand_live_countries() {
    let source = Fixture::with_countries(5_000);
    let countries = ccountry::Reader::new(&source, MODULE, source.world.game_state()).unwrap();
    let slots = countries.count() as usize;
    let mut packet = CountryMovementPacket::try_new(slots).unwrap();
    packet
        .reserve_transport(slots * source.regions.len(), source.regions.len())
        .unwrap();
    let mut output = [RegionTransportSpeed::default(); 3];
    let mut owners = 0;
    for slot in 1..countries.count() {
        let Some(owner) = countries.at(slot).unwrap() else {
            continue;
        };
        let capture = capture_transport(
            &source,
            MODULE,
            source.world.game_state(),
            owner,
            &source.regions,
            &mut output,
            &mut packet,
        );
        assert_eq!(
            (capture.len, capture.first_failure),
            (Ok(3), None),
            "owner {}",
            owner.id
        );
        assert_eq!(output.map(|row| row.speed), [Some(200_000); 3]);
        assert_eq!(output.map(|row| row.failure), [None; 3]);
        owners += 1;
    }
    // The fixture's raw three aliases canonical two; every other owner remains.
    assert_eq!(owners, 4_999);
}

#[test]
fn diplomacy_root_fault_does_not_deny_transport() {
    let source = Fixture::new();
    let owner = source.owner();
    let memory = observed(&source, Some(owner.pointer + ccountry::DIPLOMACY_STATUS));
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let mut transport = [RegionTransportSpeed::default(); 3];
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        owner,
        &source.regions,
        &mut transport,
        &mut packet,
    );
    assert_eq!(capture.len, Ok(3));
    assert_eq!(capture.first_failure, None);
    assert_eq!(transport.map(|row| row.speed), [Some(200_000); 3]);
    packet.clear();
}

#[test]
fn transport_fault_preserves_region_ids_and_later_queries() {
    let source = Fixture::new();
    let memory = observed(&source, Some(source.regions[1].pointer));
    let mut output = [RegionTransportSpeed::default(); 3];
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let capture = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        source.owner(),
        &source.regions,
        &mut output,
        &mut packet,
    );
    assert_eq!((capture.len, capture.required), (Ok(3), 3));
    assert!(capture.first_failure.is_some());
    assert_eq!(output.map(|row| row.region), [7, 29, 86]);
    assert_eq!(
        output.map(|row| row.speed),
        [Some(200_000), None, Some(200_000)]
    );
    packet.clear();
}

#[test]
fn empty_domains_are_lazy_and_zero_owner_is_an_explicit_refusal() {
    let source = Fixture::new();
    let memory = observed(
        &source,
        Some(source.owner().pointer + ccountry::DIPLOMACY_STATUS),
    );
    let countries = ccountry::Reader::new(&memory, MODULE, source.world.game_state()).unwrap();
    let owner = countries.resolve(1).unwrap();
    memory.reads.borrow_mut().clear();
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let transport = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        owner,
        &[],
        &mut [],
        &mut packet,
    );
    assert_eq!(transport.len, Ok(0));
    let zero = ccountry::Country {
        pointer: owner.pointer,
        canonical_slot: 0,
        id: 0,
    };
    assert!(
        capture_transport(
            &memory,
            MODULE,
            source.world.game_state(),
            zero,
            &[],
            &mut [],
            &mut packet,
        )
        .len
        .is_err()
    );
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn capacity_refusals_preserve_full_mod_sized_domain_without_native_reads() {
    let source = Fixture::new();
    let memory = observed(&source, None);
    let countries = ccountry::Reader::new(&memory, MODULE, source.world.game_state()).unwrap();
    let owner = countries.resolve(1).unwrap();
    memory.reads.borrow_mut().clear();
    let regions: Vec<TransportRegion> = (0..5_000)
        .map(|id| TransportRegion { id, pointer: 0 })
        .collect();
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let transport = capture_transport(
        &memory,
        MODULE,
        source.world.game_state(),
        owner,
        &regions,
        &mut [],
        &mut packet,
    );
    assert_eq!(
        (transport.len, transport.required),
        (Err(TRANSPORT_CAPACITY), 5_000)
    );
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn duplicate_domain_keys_are_refused_before_a_native_query() {
    let source = Fixture::new();
    let memory = observed(&source, None);
    let countries = ccountry::Reader::new(&memory, MODULE, source.world.game_state()).unwrap();
    let owner = countries.resolve(1).unwrap();
    memory.reads.borrow_mut().clear();
    let mut packet = CountryMovementPacket::try_new(8).unwrap();
    let repeated = [source.regions[0]; 2];
    let mut transport = [RegionTransportSpeed::default(); 2];
    assert!(
        capture_transport(
            &memory,
            MODULE,
            source.world.game_state(),
            owner,
            &repeated,
            &mut transport,
            &mut packet,
        )
        .len
        .is_err()
    );
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn region_domain_preserves_live_region_zero_and_skips_nullable_slots() {
    let mut source = Fixture::new();
    let state = source.world.game_state();
    let array = Array::read(&source, state + cgame_state::STRATEGIC_REGIONS, 8).unwrap();
    let zero = source.world.alloc(0x180);
    source.world.u64(array.data, zero as u64);
    source
        .world
        .u64(zero, (MODULE + cstrategic_region::VTABLE_RVA) as u64);
    source.world.u32(zero + cstrategic_region::ID, 0);
    source.world.u32(zero + cstrategic_region::RUNTIME_SLOT, 0);
    let mut regions = [TransportRegion::default(); 4];
    let capture = capture_region_domain(&source, MODULE, state, &mut regions);
    assert_eq!((capture.len, capture.required), (Ok(4), 4));
    assert_eq!(regions.map(|region| region.id), [0, 7, 29, 86]);
    assert_eq!(regions[0].pointer, zero);
    // A null slot is absent, not a refusal, and still counts toward the bound.
    source.world.u64(array.data, 0);
    let mut regions = [TransportRegion::default(); 4];
    let capture = capture_region_domain(&source, MODULE, state, &mut regions);
    assert_eq!((capture.len, capture.required), (Ok(3), 4));
    assert_eq!(
        regions[..3]
            .iter()
            .map(|region| region.id)
            .collect::<Vec<_>>(),
        [7, 29, 86]
    );
}

#[test]
fn region_domain_shortage_and_failed_hint_keep_the_full_header_bound() {
    let source = Fixture::new();
    let state = source.world.game_state();
    let memory = observed(&source, None);
    let regions = capture_region_domain(&memory, MODULE, state, &mut []);
    assert_eq!(regions.required, 4);
    assert!(regions.len.is_err());
    assert_eq!(memory.reads.borrow().len(), 1);
    assert_eq!(region_demand(&memory, state), Ok(4));
    let memory = observed(&source, Some(state + cgame_state::STRATEGIC_REGIONS));
    assert!(region_demand(&memory, state).is_err());
}

#[test]
fn region_domain_refuses_duplicate_ids_and_accepts_a_valid_empty_census() {
    let mut source = Fixture::new();
    let state = source.world.game_state();
    source.world.u32(
        source.regions[1].pointer + cstrategic_region::ID,
        source.regions[0].id,
    );
    let mut regions = [TransportRegion::default(); 4];
    let capture = capture_region_domain(&source, MODULE, state, &mut regions);
    assert_eq!(
        capture.len,
        Err("country movement region IDs are not unique")
    );
    assert_eq!(capture.required, 4);
    assert!(regions.iter().all(|region| region.pointer == 0));
    source
        .world
        .array(state + cgame_state::STRATEGIC_REGIONS, 0, 0);
    let capture = capture_region_domain(&source, MODULE, state, &mut []);
    assert_eq!((capture.len, capture.required), (Ok(0), 0));
}
