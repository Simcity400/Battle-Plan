//! Synthetic original census fixtures for the selected transport borrower.
use super::*;
use std::cell::{Cell, RefCell};

struct ObservedMemory {
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Memory for ObservedMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        false
    }
}

struct CaptureFixture {
    source: creference_registry::tests::Fixture,
    reads: RefCell<Vec<(usize, usize)>>,
    fault_once: Cell<Option<usize>>,
}
impl CaptureFixture {
    fn put(&mut self, address: usize, value: &[u8]) {
        self.source.put(address, value);
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + crate::cpdx_array::CAPACITY, &count.to_le_bytes());
        self.put(address + crate::cpdx_array::COUNT, &count.to_le_bytes());
    }
    fn attempts(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|(at, len)| (*at..at + len).contains(&address))
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
    carmy::DivisionSeed {
        division,
        pointer,
        handle: Some(division),
        class: Some(0),
        primary_vtable: Some(carmy::VTABLE_RVA as u64),
        secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA as u64),
        ..Default::default()
    }
}

struct Fixture {
    memory: CaptureFixture,
    seeds: [carmy::DivisionSeed; 2],
    wrapper: usize,
    template_handle: u64,
    stat: usize,
}

/// Two divisions sharing one stat block and one registered template. The
/// stat block also holds the former selected-attack scalars and terrain
/// adjusters, which no reader here may touch.
fn fixture() -> Fixture {
    const STAT: usize = 0xc000;
    const WRAPPER: usize = 0xd000;
    let template_handle = (23_u64 << 32) | u64::from(template::REFERENCE_TYPE);
    let mut registry = creference_registry::tests::Fixture::new(template::REFERENCE_TYPE);
    registry.entry(
        creference_registry::tests::Fixture::index(template_handle),
        1,
        template_handle,
        WRAPPER as u64,
    );
    let root = creference_registry::LOW_ROOTS_RVA + template::REFERENCE_TYPE as usize * 8;
    let table = crate::read::<u64>(&registry, root).unwrap() as usize;
    registry.put(table + creference_registry::COUNT, &1_u32.to_le_bytes());
    let mut memory = CaptureFixture {
        source: registry,
        reads: RefCell::new(Vec::new()),
        fault_once: Cell::new(None),
    };
    memory.put(STAT, &(stats::VTABLE_RVA as u64).to_le_bytes());
    for offset in [0x390, 0x398, 0x368, 0x370, 0x3e0, 0x340] {
        memory.put(STAT + offset, &(-9_i64).to_le_bytes());
    }
    memory.array(STAT + stats::MOVEMENT_TERRAIN_ADJUSTERS, 0xe000, 7);
    memory.put(WRAPPER, &(template::VTABLE_RVA as u64).to_le_bytes());
    memory.put(WRAPPER + template::TYPE, &template_handle.to_le_bytes());
    let seeds = [(9_u64 << 32) | 51, (7_u64 << 32) | 51].map(|division| carmy::DivisionSeed {
        stat_block: Some(STAT),
        standard_template: Some(WRAPPER),
        ..class_zero_seed(division, 0xf000 + (division >> 32) as usize * 0x100)
    });
    Fixture {
        memory,
        seeds,
        wrapper: WRAPPER,
        template_handle,
        stat: STAT,
    }
}

#[test]
fn zero_unit_sample_reads_nothing() {
    let memory = ObservedMemory {
        reads: RefCell::new(Vec::new()),
    };
    let mut packet = Packet::try_new(Capacity::default()).unwrap();
    packet.capture(&memory, 0, 42, &[], Outcome::Rows { len: 0 });
    assert_eq!(packet.outcome(), Outcome::Rows { len: 0 });
    assert!(packet.originals().is_empty());
    assert!(memory.reads.borrow().is_empty());
}

/// Each shared stat block costs its vtable read only: the selected-attack
/// scalars, terrain adjusters and acclimatization are no longer acquired.
#[test]
fn capture_authenticates_shared_stat_and_template_once_without_attack_reads() {
    let fixture = fixture();
    let mut packet = Packet::try_new(Capacity { rows: 2 }).unwrap();
    let allocated = packet.allocated_bytes();
    packet.capture(
        &fixture.memory,
        0,
        -17,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
    );
    assert_eq!(packet.outcome(), Outcome::Rows { len: 2 });
    assert_eq!(packet.required(), Capacity { rows: 2 });
    assert_eq!(packet.allocated_bytes(), allocated);
    assert_eq!(
        packet
            .originals()
            .iter()
            .map(|row| row.division)
            .collect::<Vec<_>>(),
        vec![fixture.seeds[1].division, fixture.seeds[0].division]
    );
    for original in packet.originals() {
        assert_eq!(original.sample_hour, -17);
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
    for offset in [0x390, 0x398, 0x368, 0x370, 0x3e0, 0x340] {
        assert_eq!(fixture.memory.attempts(fixture.stat + offset), 0);
    }
    assert_eq!(
        fixture
            .memory
            .attempts(fixture.stat + stats::MOVEMENT_TERRAIN_ADJUSTERS),
        0
    );
    for seed in &fixture.seeds {
        assert_eq!(fixture.memory.attempts(seed.pointer), 0);
    }
    let reads = fixture.memory.reads.borrow().len();
    packet.finish_native();
    assert!(packet.originals().is_empty());
    assert_eq!(fixture.memory.reads.borrow().len(), reads);
}

#[test]
fn shared_stat_fault_is_one_observation_for_both_aliases() {
    let fixture = fixture();
    fixture.memory.fault_once.set(Some(fixture.stat));
    let mut packet = Packet::try_new(Capacity { rows: 2 }).unwrap();
    packet.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
    );
    assert_eq!(packet.outcome(), Outcome::Rows { len: 2 });
    for original in packet.originals() {
        assert_eq!(
            original.stat.map(|stat| stat.pointer),
            Err("selected unit original field unreadable")
        );
        assert_eq!(
            original.template.map(|template| template.handle),
            Ok(fixture.template_handle)
        );
    }
    assert_eq!(fixture.memory.attempts(fixture.stat), 1);
    // A fresh hour retires the refusal; the same hour never retries it.
    packet.capture(
        &fixture.memory,
        0,
        45,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
    );
    assert!(
        packet
            .originals()
            .iter()
            .all(|row| row.sample_hour == 45 && row.stat.is_ok())
    );
    assert_eq!(fixture.memory.attempts(fixture.stat), 2);
}

#[test]
fn an_unauthenticated_seed_ends_the_pass_with_a_sorted_prefix() {
    let fixture = fixture();
    let mut seeds = fixture.seeds.to_vec();
    let mut invalid = class_zero_seed((11_u64 << 32) | 51, 0xff00);
    invalid.class = Some(1);
    seeds.push(invalid);
    let mut packet = Packet::try_new(Capacity { rows: 3 }).unwrap();
    packet.capture(&fixture.memory, 0, 72, &seeds, Outcome::Rows { len: 3 });
    assert_eq!(
        packet.outcome(),
        Outcome::Unavailable {
            reason: "selected unit original class0 seed identity unavailable"
        }
    );
    assert!(
        packet
            .originals()
            .windows(2)
            .all(|pair| pair[0].division < pair[1].division)
    );
    for seed in &seeds[..2] {
        assert_eq!(
            packet.original(seed.division).unwrap().pointer,
            seed.pointer
        );
    }
    assert!(packet.original(invalid.division).is_none());
}

#[test]
fn malformed_template_identity_is_refused_and_memoized_before_registry_lookup() {
    let wrapper = 0x1000;
    let mut memory = fixture().memory;
    memory.reads.borrow_mut().clear();
    memory.put(wrapper, &(template::VTABLE_RVA as u64).to_le_bytes());
    memory.put(
        wrapper + template::TYPE,
        &(template::REFERENCE_TYPE as u64).to_le_bytes(),
    );
    let mut packet = Packet::try_new(Capacity { rows: 2 }).unwrap();
    let mut resolver = creference_registry::ScopedResolver::new(0);
    let failure = packet.template(&memory, 0, Ok(wrapper), &mut resolver);
    assert!(matches!(
        failure,
        Err("selected unit original template full handle invalid")
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

/// Repeated templates are found by pointer: one probe per repeat instead of
/// a scan of every earlier template, a 5,000-template census included.
#[test]
fn repeated_templates_are_found_by_pointer_without_scanning_or_rereading() {
    const TEMPLATES: usize = 5_000;
    let memory = ObservedMemory {
        reads: RefCell::new(Vec::new()),
    };
    let mut packet = Packet::try_new(Capacity { rows: TEMPLATES }).unwrap();
    let mut resolver = creference_registry::ScopedResolver::new(0);
    let pointers = (0..TEMPLATES).map(|index| 0x10_0000 + index * 0x100);
    let first: Vec<_> = pointers
        .clone()
        .map(|pointer| packet.template(&memory, 0, Ok(pointer), &mut resolver))
        .map(|observed| observed.map(|template| (template.wrapper, template.handle)))
        .collect();
    assert!(first.iter().all(Result::is_err));
    let reads = memory.reads.borrow().len();
    assert_eq!(packet.template_probes, 0);
    let again: Vec<_> = pointers
        .rev()
        .map(|pointer| packet.template(&memory, 0, Ok(pointer), &mut resolver))
        .map(|observed| observed.map(|template| (template.wrapper, template.handle)))
        .collect();
    assert_eq!(again.into_iter().rev().collect::<Vec<_>>(), first);
    assert_eq!(memory.reads.borrow().len(), reads);
    assert_eq!(packet.template_probes, TEMPLATES);
    packet.clear();
    assert!(packet.template_indices.is_empty() && packet.template_probes == 0);
}

#[test]
fn transport_borrows_original_success_and_fault_once_before_native_retirement() {
    use crate::selected_transport_units::Transfer;
    use crate::selected_transport_units::{Capacity as TransportCapacity, Packet as Transport};
    let mut fixture = fixture();
    fixture
        .memory
        .array(fixture.wrapper + template::SUBUNIT_COUNTS, 0x31000, 0);
    fixture
        .memory
        .put(fixture.stat + 0x2b8, &(-123i64).to_le_bytes());
    for seed in &mut fixture.seeds {
        seed.naval_transfer_membership = Some(0);
    }
    let mut originals = Packet::try_new(Capacity { rows: 2 }).unwrap();
    originals.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
    );
    fixture.memory.fault_once.set(Some(fixture.stat + 0x2b8));
    let mut transport = Transport::try_new(TransportCapacity {
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
fn transport_refuses_a_stale_original_sample() {
    use crate::selected_transport_units::{Capacity as TransportCapacity, Packet as Transport};
    let mut fixture = fixture();
    fixture
        .memory
        .array(fixture.wrapper + template::SUBUNIT_COUNTS, 0x31000, 0);
    for seed in &mut fixture.seeds {
        seed.naval_transfer_membership = Some(0);
    }
    let mut originals = Packet::try_new(Capacity { rows: 2 }).unwrap();
    originals.capture(
        &fixture.memory,
        0,
        44,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
    );
    let mut transport = Transport::try_new(TransportCapacity {
        rows: 2,
        members: 0,
    })
    .unwrap();
    transport.capture(
        &fixture.memory,
        0,
        45,
        &fixture.seeds,
        Outcome::Rows { len: 2 },
        &originals,
    );
    assert!(transport.rows().iter().all(|row| row.coastal
        == Err("selected transport original sample is stale")
        && row.weight == Err("selected transport original sample is stale")));
}
