use super::*;
use crate::selected_attack::{StatObservation, TemplateObservation};
use std::{cell::RefCell, collections::BTreeMap};

const WRAPPER: usize = 0x1000;
const STAT: usize = 0x3000;
const PAIRS: usize = 0x5000;
const MEMBER: usize = 0x7000;

#[derive(Default)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
    calls: RefCell<Vec<(usize, usize)>>,
}
impl Fixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.bytes.insert(address + offset, *byte);
        }
    }
    fn composition(&mut self, count: u32) {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&(PAIRS as u64).to_le_bytes());
        bytes[8..12].copy_from_slice(&count.to_le_bytes());
        bytes[12..].copy_from_slice(&count.to_le_bytes());
        self.put(
            WRAPPER + crate::creferenced_division_template::COMPOSITION + GROUPED_MEMBERS,
            &bytes,
        );
    }
    fn member(&mut self, pointer: usize, flag: u8, token: Option<i32>) {
        self.put(
            pointer,
            &(crate::csub_unit_definition::VTABLE_RVA as u64).to_le_bytes(),
        );
        self.put(pointer + COASTAL_FLAG, &[flag]);
        if let Some(token) = token {
            self.put(pointer + CATEGORY_TOKEN, &token.to_le_bytes());
        }
    }
}
impl Memory for Fixture {
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
fn original() -> OriginalObservation {
    OriginalObservation {
        division: 1,
        pointer: 0x9000,
        stat: Ok(StatObservation { pointer: STAT }),
        template: Ok(TemplateObservation {
            wrapper: WRAPPER,
            handle: (2u64 << 32) | 52,
        }),
    }
}

fn with_memo(run: impl FnOnce(&mut Memo<'_>)) {
    let mut templates = [TemplateMemo::default(); 2];
    let mut members = [MemberMemo::default(); 4];
    {
        let mut memo = Memo::new(&mut templates, &mut members);
        run(&mut memo);
    }
    assert!(
        templates
            .iter()
            .all(|entry| entry.wrapper == 0 && entry.handle == 0)
    );
    assert!(members.iter().all(|entry| entry.pointer == 0));
}

#[test]
fn original_certificate_failures_never_retry_stat_or_template_selectors() {
    let memory = Fixture::default();
    let mut source = original();
    source.stat = Err("original stat refusal");
    source.template = Err("original template refusal");
    with_memo(|memo| {
        assert_eq!(
            memo.capture(&memory, 0, &source),
            Err("original template refusal")
        )
    });
    assert!(memory.calls.borrow().is_empty());
}

#[test]
fn empty_contained_composition_is_true_without_template_vtable_retry() {
    let mut memory = Fixture::default();
    memory.composition(0);
    memory.put(STAT + WEIGHT, &i64::MIN.to_le_bytes());
    with_memo(|memo| assert_eq!(memo.capture(&memory, 0, &original()), Ok(true)));
    assert_eq!(memory.calls.borrow().len(), 1);
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|(address, _)| *address == STAT || *address == WRAPPER)
    );
}

#[test]
fn composition_stride16_and_nonzero_flag_do_not_demand_category_or_wrapper_vector() {
    let mut memory = Fixture::default();
    memory.composition(2);
    memory.put(STAT + WEIGHT, &123i64.to_le_bytes());
    memory.put(PAIRS, &(MEMBER as u64).to_le_bytes());
    memory.put(PAIRS + STRIDE, &((MEMBER + 0x1000) as u64).to_le_bytes());
    memory.member(MEMBER, 7, None);
    memory.member(MEMBER + 0x1000, 0, Some(COASTAL_ALTERNATIVE_TOKEN));
    with_memo(|memo| assert_eq!(memo.capture(&memory, 0, &original()), Ok(true)));
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|(address, _)| *address == MEMBER + CATEGORY_TOKEN
                || *address == WRAPPER + GROUPED_MEMBERS
                || *address == PAIRS + 8)
    );
}

#[test]
fn native_first_false_skips_unreadable_later_members() {
    let mut memory = Fixture::default();
    memory.composition(2);
    memory.put(PAIRS, &(MEMBER as u64).to_le_bytes());
    memory.member(MEMBER, 0, Some(0));
    with_memo(|memo| assert_eq!(memo.capture(&memory, 0, &original()), Ok(false)));
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|(address, _)| *address == PAIRS + STRIDE)
    );
}

#[test]
fn both_success_and_failure_memos_are_pass_local_and_do_not_retry() {
    let mut memory = Fixture::default();
    memory.composition(1);
    memory.put(PAIRS, &(MEMBER as u64).to_le_bytes());
    memory.member(MEMBER, 0, None);
    let mut source = original();
    source.stat = Err("original stat refusal");
    with_memo(|memo| {
        let first = memo.capture(&memory, 0, &source);
        let count = memory.calls.borrow().len();
        assert!(first.is_err());
        // Another original unit shares the refused template. Its independent
        // original stat refusal entails no attempted weight observation.
        source.division += 1;
        assert_eq!(memo.capture(&memory, 0, &source), first);
        assert_eq!(memory.calls.borrow().len(), count);
    });
}

#[test]
fn short_template_storage_is_sticky_without_native_leaf_reads() {
    let mut memory = Fixture::default();
    memory.put(STAT + WEIGHT, &123i64.to_le_bytes());
    let mut templates = [];
    let mut members = [];
    let mut memo = Memo::new(&mut templates, &mut members);
    let first = memo.capture(&memory, 0, &original());
    let calls = memory.calls.borrow().len();
    assert!(first.is_err());
    let mut source = original();
    source.stat = Err("original stat refusal");
    assert_eq!(memo.capture(&memory, 0, &source), first);
    assert_eq!(memory.calls.borrow().len(), calls);
}

#[test]
fn transfer_uses_the_original_selector_and_reads_source_only_after_zero_status() {
    const TRANSFER: usize = 0x12000;
    const HANDLE: u64 = (7u64 << 32) | 4713;
    let mut memory = Fixture::default();
    let mut resolves = 0;
    assert_eq!(
        transfer(
            &memory,
            0,
            Err("original selector failed"),
            true,
            |_| panic!("unreached registry")
        ),
        Err("original selector failed")
    );
    assert_eq!(
        transfer(&memory, 0, Ok(0), true, |_| panic!("null registry")),
        Ok(Transfer::Absent)
    );
    assert!(memory.calls.borrow().is_empty());
    memory.put(
        TRANSFER,
        &(crate::cnaval_unit_transfer::VTABLE_RVA as u64).to_le_bytes(),
    );
    memory.put(
        TRANSFER + crate::cnaval_unit_transfer::HANDLE,
        &HANDLE.to_le_bytes(),
    );
    memory.put(
        TRANSFER + crate::cnaval_unit_transfer::ACTION_STATUS,
        &(-7i32).to_le_bytes(),
    );
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), true, |handle| {
            assert_eq!(handle, HANDLE);
            resolves += 1;
            Ok(Some(TRANSFER))
        }),
        Ok(Transfer::Present {
            handle: HANDLE,
            status: Ok(-7),
            source: Err("selected transport transfer source unreached")
        })
    );
    assert_eq!(resolves, 1);
    assert!(!memory.calls.borrow().iter().any(|(address, _)| *address == TRANSFER + crate::cnaval_unit_transfer::SOURCE_PROVINCE));
    memory.put(
        TRANSFER + crate::cnaval_unit_transfer::ACTION_STATUS,
        &0i32.to_le_bytes(),
    );
    memory.put(
        TRANSFER + crate::cnaval_unit_transfer::SOURCE_PROVINCE,
        &42u32.to_le_bytes(),
    );
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), true, |_| Ok(Some(TRANSFER))),
        Ok(Transfer::Present {
            handle: HANDLE,
            status: Ok(0),
            source: Ok(42)
        })
    );
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), false, |_| Ok(Some(TRANSFER))),
        Ok(Transfer::Present {
            handle: HANDLE,
            status: Ok(0),
            source: Err("selected transport transfer source unreached")
        })
    );
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(
                |(address, _)| *address == TRANSFER + crate::cnaval_unit_transfer::SOURCE_PROVINCE
            )
            .count(),
        1
    );
}

#[test]
fn transfer_authenticated_identity_survives_independent_status_or_source_refusal() {
    const TRANSFER: usize = 0x12000;
    const HANDLE: u64 = (7u64 << 32) | 4713;
    let mut memory = Fixture::default();
    memory.put(
        TRANSFER,
        &(crate::cnaval_unit_transfer::VTABLE_RVA as u64).to_le_bytes(),
    );
    memory.put(
        TRANSFER + crate::cnaval_unit_transfer::HANDLE,
        &HANDLE.to_le_bytes(),
    );
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), true, |_| Ok(Some(TRANSFER))),
        Ok(Transfer::Present {
            handle: HANDLE,
            status: Err("selected transport transfer status unreadable"),
            source: Err("selected transport transfer source unreached")
        })
    );
    assert!(!memory.calls.borrow().iter().any(|(address, _)| *address == TRANSFER + crate::cnaval_unit_transfer::SOURCE_PROVINCE));
    memory.put(
        TRANSFER + crate::cnaval_unit_transfer::ACTION_STATUS,
        &0i32.to_le_bytes(),
    );
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), true, |_| Ok(Some(TRANSFER))),
        Ok(Transfer::Present {
            handle: HANDLE,
            status: Ok(0),
            source: Err("selected transport transfer source unreadable")
        })
    );
    assert_eq!(
        transfer(&memory, 0, Ok(TRANSFER), true, |_| Ok(Some(TRANSFER + 1))),
        Err("selected transport transfer registry mismatch")
    );
}

#[test]
fn prepared_packet_shortage_never_reads_and_reservation_does_not_shrink_or_keep_native_keys() {
    let memory = Fixture::default();
    let original = crate::selected_attack::UnitsPacket::vacant();
    let seed = crate::carmy::DivisionSeed {
        division: 1,
        pointer: 1,
        ..Default::default()
    };
    let mut packet = Packet::vacant();
    packet.capture(
        &memory,
        0,
        7,
        &[seed],
        crate::walk::Outcome::Rows { len: 1 },
        &original,
    );
    assert_eq!(packet.required().rows, 1);
    assert!(matches!(
        packet.outcome(),
        crate::walk::Outcome::Unavailable { .. }
    ));
    assert!(packet.rows().is_empty() && memory.calls.borrow().is_empty());
    packet
        .reserve(Capacity {
            rows: 4,
            members: 8,
        })
        .unwrap();
    let allocated = packet.allocated_bytes();
    packet
        .reserve(Capacity {
            rows: 1,
            members: 0,
        })
        .unwrap();
    assert_eq!((packet.templates.len(), packet.members.len()), (4, 8));
    packet.capture(
        &memory,
        0,
        8,
        &[seed],
        crate::walk::Outcome::Rows { len: 1 },
        &original,
    );
    assert_eq!(packet.outcome(), crate::walk::Outcome::Rows { len: 1 });
    assert!(
        packet.rows()[0].weight.is_err()
            && packet.rows()[0].coastal.is_err()
            && packet.rows()[0].transfer.is_err()
    );
    assert!(packet.weights.is_empty() && packet.transfers.is_empty());
    assert!(packet.templates.iter().all(|entry| entry.wrapper == 0));
    assert!(packet.members.iter().all(|entry| entry.pointer == 0));
    assert_eq!(packet.allocated_bytes(), allocated);
    packet.fail("terminal restore failed");
    assert_eq!(packet.required().rows, 1);
    assert_eq!(
        packet.outcome(),
        crate::walk::Outcome::Unavailable {
            reason: "terminal restore failed"
        }
    );
    assert!(packet.rows().is_empty() && packet.weights.is_empty() && packet.transfers.is_empty());
    packet.clear();
    assert!(packet.rows().is_empty());
    assert_eq!(packet.required(), Capacity::default());
}

#[test]
fn worker_grouped_members_grow_from_original_header_and_do_not_repeat_aliases() {
    let mut templates = Vec::new();
    let mut members = Vec::new();
    for count in [5000, 11003] {
        let mut memory = Fixture::default();
        memory.composition(count);
        for index in 0..count {
            let pointer = 0x10_0000 + index as usize * 0x1000;
            memory.put(
                PAIRS + index as usize * STRIDE,
                &(pointer as u64).to_le_bytes(),
            );
            memory.member(pointer, 1, None);
        }
        {
            let mut fixed = Memo::new(&mut [], &mut []);
            assert_eq!(
                fixed.capture(&memory, 0, &original()),
                Err("selected transport template arena exhausted")
            );
        }
        {
            let mut memo = Memo::growing(&mut templates, &mut members);
            assert_eq!(memo.capture(&memory, 0, &original()), Ok(true));
            let reads = memory.calls.borrow().len();
            assert_eq!(memo.capture(&memory, 0, &original()), Ok(true));
            assert_eq!(memory.calls.borrow().len(), reads);
            assert_eq!(memo.required_members, count as usize);
        }
        assert!(members.iter().all(|entry| entry.pointer == 0));
    }
}
