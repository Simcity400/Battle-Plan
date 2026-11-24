use super::*;
use engine_native_memory::{Memory, memory::SliceMemory};
use crate::{corder_instance as order, corders_group as group};

#[test]
fn allocation_capacity_overflow_preserves_membership_packet() {
    let mut inputs = MembershipPacket::new(2, 2, 2, 2);
    let before = inputs.capacity();
    let required = MembershipCapacity {
        groups: usize::MAX,
        ..before
    };
    assert_eq!(
        inputs.grow(required).unwrap_err().kind(),
        std::io::ErrorKind::OutOfMemory,
    );
    assert_eq!(inputs.capacity(), before);
}

const MEMBER: usize = 0x1000;
const OWN: usize = 0x2000;
const PARENT: usize = 0x3000;
const ORDER: usize = 0x4000;
const ROOTS: usize = 0x5000;
const MEMBERS: usize = 0x6000;

struct Fixture {
    bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let mut result = Self {
            bytes: vec![0; 0x9000],
        };
        result.word(MEMBER + GROUP, OWN as u64);
        result.word(OWN, group::VTABLE_RVA.0 as u64);
        result.word(OWN + group::HANDLE, 0xffff_ffff_0000_0035);
        result.word(PARENT, engine_native_parts::carmy_group::VTABLE_RVA.0 as u64);
        result.word(PARENT + group::HANDLE, 0xffff_fffe_0000_0035);
        result
    }

    fn word(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, data as u64);
        self.bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
        self.bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
    }

    fn assigned(&mut self, owner: usize, offset: usize, index: usize) {
        self.array(owner + offset, ROOTS, 1);
        self.word(ROOTS, ORDER as u64);
        self.word(ORDER, order::VTABLE_RVA.0 as u64);
        self.word(ORDER + order::OWNER, owner as u64);
        self.array(
            ORDER + order::ASSIGNED_MEMBER_DATA,
            MEMBERS,
            index as u32 + 1,
        );
        self.word(MEMBERS + index * 8, MEMBER as u64);
    }
}

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, destination)
    }
}

struct Trace<'a> {
    source: &'a Fixture,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}
impl Memory for Trace<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        self.fault != Some(address) && self.source.copy(address, output)
    }
}
impl<'a> Trace<'a> {
    fn new(source: &'a Fixture) -> Self {
        Self {
            source,
            reads: Default::default(),
            fault: None,
        }
    }
    fn count(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == address)
            .count()
    }
}

fn packet(queries: &[(usize, usize)]) -> MembershipPacket {
    let mut result = MembershipPacket::new(queries.len(), 16, 16, 16);
    for &(member, own_group) in queries {
        result
            .push_query(MembershipQuery { member, own_group })
            .unwrap();
    }
    result
}

#[test]
fn packet_late_chunk_fault_preserves_earlier_matches_and_is_not_retried() {
    let mut source = Fixture::new();
    source.assigned(OWN, group::PRIMARY_ORDER_DATA, 128);
    source.word(MEMBERS, MEMBER as u64);
    source.word(MEMBER + 0x100 + GROUP, OWN as u64);
    let fault = MEMBERS + 512;
    let memory = Trace {
        fault: Some(fault),
        ..Trace::new(&source)
    };
    let mut inputs = packet(&[(MEMBER + 0x100, OWN), (MEMBER, OWN)]);
    inputs.acquire(&memory, 0);
    assert_eq!(inputs.outcome(0), Err("assigned members unreadable"));
    assert_eq!(inputs.outcome(1), Ok(true));
    assert_eq!(memory.count(fault), 1);
    assert_eq!(memory.count(MEMBERS + 1024), 0);
}

#[test]
fn packet_late_root_fault_cannot_precede_first_hit_or_be_skipped() {
    let mut source = Fixture::new();
    source.assigned(OWN, group::PRIMARY_ORDER_DATA, 0);
    source.word(MEMBER + 0x100 + GROUP, OWN as u64);
    source.array(OWN + group::PRIMARY_ORDER_DATA, ROOTS, 2);
    source.word(ROOTS + 8, 0);
    let memory = Trace::new(&source);
    let mut inputs = packet(&[(MEMBER, OWN), (MEMBER + 0x100, OWN)]);
    inputs.acquire(&memory, 0);
    assert_eq!(inputs.outcome(0), Ok(true));
    assert_eq!(inputs.outcome(1), Err("membership order identity mismatch"));
    assert_eq!(memory.count(OWN + group::SECONDARY_ORDER_DATA), 0);
    let mut one = packet(&[(MEMBER, OWN)]);
    let memory = Trace::new(&source);
    one.acquire(&memory, 0);
    assert_eq!(memory.count(ROOTS + 8), 0, "root vector has no lookahead");
    assert_eq!(one.outcome(0), Ok(true));
}

#[test]
fn packet_wrong_owner_beats_cached_member_header_failure() {
    let mut source = Fixture::new();
    source.assigned(OWN, group::PRIMARY_ORDER_DATA, 0);
    source.word(MEMBER + 0x100 + GROUP, PARENT as u64);
    source.array(PARENT + group::PRIMARY_ORDER_DATA, ROOTS + 0x100, 1);
    source.word(ROOTS + 0x100, ORDER as u64);
    source.array(ORDER + order::ASSIGNED_MEMBER_DATA, 0, 1);
    let memory = Trace::new(&source);
    let mut inputs = packet(&[(MEMBER, OWN), (MEMBER + 0x100, PARENT)]);
    inputs.acquire(&memory, 0);
    assert_eq!(inputs.outcome(0), Err("assigned members invalid"));
    assert_eq!(inputs.outcome(1), Err("membership order identity mismatch"));
    assert_eq!(memory.count(ORDER + order::ASSIGNED_MEMBER_DATA), 1);
}

#[test]
fn packet_fault_preserves_owned_prefix_without_retrying_failed_source() {
    let mut source = Fixture::new();
    source.assigned(OWN, group::PRIMARY_ORDER_DATA, 128);
    source.word(MEMBERS, MEMBER as u64);
    source.word(MEMBER + 0x100 + GROUP, OWN as u64);
    let mut inputs = packet(&[(MEMBER, OWN), (MEMBER + 0x100, OWN)]);
    let memory = Trace {
        fault: Some(MEMBERS + 1024),
        ..Trace::new(&source)
    };
    inputs.acquire(&memory, 0);
    assert_eq!(inputs.outcome(1), Err("assigned members unreadable"));
    assert_eq!(inputs.chunks.len(), 2);
    assert_eq!(&inputs.chunks[0].bytes[..8], &(MEMBER as u64).to_le_bytes());
    let before = memory.reads.borrow().len();
    assert_eq!(inputs.outcome(1), Err("assigned members unreadable"));
    assert_eq!(inputs.outcome(0), Ok(true));
    assert_eq!(memory.reads.borrow().len(), before);
    for address in [MEMBERS, MEMBERS + 512, MEMBERS + 1024] {
        assert_eq!(memory.count(address), 1);
    }
}

#[test]
fn packet_zero_and_small_capacities_preserve_original_reads_and_failures() {
    let mut source = Fixture::new();
    source.assigned(OWN, group::PRIMARY_ORDER_DATA, 64);
    source.word(MEMBERS, MEMBER as u64);
    source.word(MEMBERS + 512, (MEMBER + 0x100) as u64);
    source.word(MEMBER + 0x100 + GROUP, OWN as u64);
    let queries = [(MEMBER, OWN), (MEMBER + 0x100, OWN), (MEMBER, OWN)];
    for fault in [
        None,
        Some(MEMBER + GROUP),
        Some(OWN),
        Some(OWN + group::PARENT),
        Some(ROOTS),
        Some(ORDER),
        Some(ORDER + order::OWNER),
        Some(ORDER + order::ASSIGNED_MEMBER_DATA),
        Some(MEMBERS + 512),
    ] {
        let reference = Trace {
            fault,
            ..Trace::new(&source)
        };
        let mut ample = packet(&queries);
        ample.acquire(&reference, 0);
        for initial in [0, 1] {
            let mut inputs = MembershipPacket::new(initial, initial, initial, initial);
            for &(member, own_group) in &queries {
                inputs
                    .push_query(MembershipQuery { member, own_group })
                    .unwrap();
            }
            let memory = Trace {
                fault,
                ..Trace::new(&source)
            };
            inputs.acquire(&memory, 0);
            for index in 0..queries.len() {
                assert_eq!(inputs.outcome(index), ample.outcome(index));
            }
            assert_eq!(*memory.reads.borrow(), *reference.reads.borrow());
            assert!(!inputs.overflowed());
            if fault == Some(MEMBERS + 512) {
                assert_eq!(inputs.required_capacity().chunks, 2);
                assert_eq!(inputs.chunks.len(), 1);
                assert!(inputs.capacity().chunks >= 2);
            }
            let allocated = inputs.allocated_bytes();
            inputs.grow(inputs.required_capacity()).unwrap();
            assert!(inputs.allocated_bytes() >= allocated);
        }
    }
}

#[test]
fn packet_index_growth_preserves_earlier_aliases_and_failure_frontiers() {
    let mut source = Fixture::new();
    let mut queries = Vec::new();
    for index in 0..6 {
        let own = OWN + index * 0x200;
        let member = MEMBER + index * 0x100;
        let order = ORDER + index * 0x400;
        let roots = 0x7000 + index * 0x20;
        source.word(member + GROUP, own as u64);
        source.word(own, group::VTABLE_RVA.0 as u64);
        source.word(own + group::HANDLE, group::REFERENCE_TYPE as u64);
        source.array(own + group::PRIMARY_ORDER_DATA, roots, 2);
        source.word(roots, order as u64);
        source.word(roots + 8, if index == 5 { ORDER } else { order } as u64);
        source.word(order, order::VTABLE_RVA.0 as u64);
        source.word(order + order::OWNER, own as u64);
        queries.push((member, own));
    }
    queries.push(queries[0]);
    let reference = Trace::new(&source);
    let mut ample = packet(&queries);
    ample.acquire(&reference, 0);
    let mut inputs = MembershipPacket::new(0, 0, 0, 0);
    for &(member, own_group) in &queries {
        inputs
            .push_query(MembershipQuery { member, own_group })
            .unwrap();
    }
    let memory = Trace::new(&source);
    inputs.acquire(&memory, 0);
    assert_eq!(*memory.reads.borrow(), *reference.reads.borrow());
    for index in 0..queries.len() {
        assert_eq!(inputs.outcome(index), ample.outcome(index));
    }
    assert_eq!(inputs.outcome(5), Err("membership order identity mismatch"));
    assert_eq!(inputs.outcome(6), Ok(false));
    assert_eq!(memory.count(ORDER), 1);
    assert_eq!(inputs.required_capacity().groups, 6);
    assert_eq!(inputs.required_capacity().roots, 12);
    assert_eq!(inputs.required_capacity().chunks, 0);
}

#[test]
fn packet_matches_scalar_branch_and_static_fault_outcomes() {
    for owner in [OWN, PARENT] {
        for offset in [group::PRIMARY_ORDER_DATA, group::SECONDARY_ORDER_DATA] {
            let mut source = Fixture::new();
            source.word(OWN + group::PARENT, PARENT as u64);
            source.assigned(owner, offset, 64);
            let direct = Trace::new(&source);
            let expected = has_order(&direct, 0, MEMBER);
            let mut inputs = packet(&[(MEMBER, OWN)]);
            inputs.acquire(&source, 0);
            assert_eq!(inputs.outcome(0), expected);
            let addresses: std::collections::BTreeSet<_> = direct
                .reads
                .borrow()
                .iter()
                .map(|&(at, _)| at)
                .filter(|&at| at != MEMBER + GROUP)
                .collect();
            for address in addresses {
                let direct = Trace {
                    fault: Some(address),
                    ..Trace::new(&source)
                };
                let memory = Trace {
                    fault: Some(address),
                    ..Trace::new(&source)
                };
                let mut inputs = packet(&[(MEMBER, OWN)]);
                inputs.acquire(&memory, 0);
                assert_eq!(
                    inputs.outcome(0),
                    has_order(&direct, 0, MEMBER),
                    "fault {address:x}"
                );
                assert_eq!(
                    memory.count(address),
                    1,
                    "no retry of failed source {address:x}"
                );
            }
        }
    }
}

#[test]
fn has_order_requires_membership_and_ignores_both_dirty_and_clean_caches() {
    let mut memory = Fixture::new();
    for dirty in [0, 1] {
        memory.bytes[MEMBER + 0x28] = dirty;
        memory.word(MEMBER + 0x18, ORDER as u64);
        memory.word(MEMBER + 0x20, ORDER as u64);
        assert_eq!(has_order(&memory, 0, MEMBER), Ok(false));
    }
    memory.assigned(OWN, group::PRIMARY_ORDER_DATA, 0);
    for dirty in [0, 1] {
        memory.bytes[MEMBER + 0x28] = dirty;
        memory.word(MEMBER + 0x18, 0);
        memory.word(MEMBER + 0x20, 0);
        assert_eq!(has_order(&memory, 0, MEMBER), Ok(true));
    }
    memory.word(MEMBERS, (MEMBER - HOST_OFFSET) as u64);
    assert_eq!(has_order(&memory, 0, MEMBER), Ok(false));
    memory.word(MEMBER + GROUP, 0);
    assert_eq!(has_order(&memory, 0, MEMBER), Ok(false));
}

#[test]
fn has_order_searches_primary_fallback_and_immediate_parent_across_batches() {
    for owner in [OWN, PARENT] {
        for offset in [group::PRIMARY_ORDER_DATA, group::SECONDARY_ORDER_DATA] {
            let mut memory = Fixture::new();
            memory.word(OWN + group::PARENT, PARENT as u64);
            memory.assigned(owner, offset, 64);
            assert_eq!(has_order(&memory, 0, MEMBER), Ok(true));
            memory.word(MEMBERS + 64 * 8, 0);
            assert_eq!(has_order(&memory, 0, MEMBER), Ok(false));
        }
    }
    let mut memory = Fixture::new();
    memory.assigned(OWN, group::TERTIARY_ORDER_DATA, 0);
    assert_eq!(has_order(&memory, 0, MEMBER), Ok(false));
}

#[test]
fn has_order_rejects_bad_identities_headers_and_overflow_without_absence() {
    for (address, value) in [
        (OWN, 1),
        (OWN + group::HANDLE, 0xffff_ffff_0000_0034),
        (ORDER, 1),
        (ORDER + order::OWNER, PARENT as u64),
        (ROOTS, 0),
        (ORDER + order::ASSIGNED_MEMBER_DATA, u64::MAX),
    ] {
        let mut memory = Fixture::new();
        memory.assigned(OWN, group::PRIMARY_ORDER_DATA, 0);
        memory.word(address, value);
        assert!(has_order(&memory, 0, MEMBER).is_err(), "{address:x}");
    }
    let mut memory = Fixture::new();
    memory.assigned(OWN, group::PRIMARY_ORDER_DATA, 0);
    memory.bytes[ORDER + order::ASSIGNED_MEMBER_DATA + 8..ORDER + order::ASSIGNED_MEMBER_DATA + 12]
        .fill(0);
    assert!(has_order(&memory, 0, MEMBER).is_err());
    memory.word(OWN + group::PARENT, PARENT as u64);
    memory.word(PARENT, 1);
    memory.array(OWN + group::PRIMARY_ORDER_DATA, 0, 0);
    assert!(has_order(&memory, 0, MEMBER).is_err());
    assert!(has_order(&memory, 0, usize::MAX).is_err());
    assert!(has_order(&memory, usize::MAX, MEMBER).is_err());
}

