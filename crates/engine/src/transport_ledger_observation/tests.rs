use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

#[derive(Default)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fail_at: Option<usize>,
    fail_once: bool,
    failed: Cell<bool>,
    partial: bool,
}

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fail_at == Some(address) && (!self.fail_once || !self.failed.replace(true)) {
            if self.partial {
                output[0] = 0xff;
            }
            return false;
        }
        for (index, byte) in output.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

impl Fixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes.extend(
            bytes
                .iter()
                .enumerate()
                .map(|(index, byte)| (address + index, *byte)),
        );
    }

    fn calls(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == address)
            .count()
    }
}

fn pointers(values: &[u64]) -> Fixture {
    let mut memory = Fixture::default();
    for (index, value) in values.iter().enumerate() {
        memory.put(0x1000 + index * 8, &value.to_le_bytes());
    }
    memory
}

fn header(task: usize, vtable: u64) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&vtable.to_le_bytes());
    bytes[8..].copy_from_slice(&(task as u64).to_le_bytes());
    bytes
}

#[test]
fn native_null_order_and_repeated_occurrences_survive_one_pointer_walk() {
    let memory = pointers(&[0x2000, 0, 0x2000]);
    let mut output = [0; 3];
    let observed = observe_ledger(
        &memory,
        Array {
            data: 0x1000,
            count: 3,
        },
        &mut output,
    );
    assert_eq!(observed.nullable, Ok(3));
    assert_eq!(
        observed.strict,
        Err("navy ledger mission unreadable or null")
    );
    assert_eq!(output, [0x2000, 0, 0x2000]);
    assert_eq!(memory.reads.borrow().len(), 3);
}

#[test]
fn native_nonnull_and_empty_lists_keep_the_old_strict_result() {
    let memory = pointers(&[1, 2, 1]);
    let mut output = [0; 3];
    let observed = observe_ledger(
        &memory,
        Array {
            data: 0x1000,
            count: 3,
        },
        &mut output,
    );
    assert_eq!((observed.nullable, observed.strict), (Ok(3), Ok(3)));
    let observed = observe_ledger(&memory, Array { data: 0, count: 0 }, &mut []);
    assert_eq!((observed.nullable, observed.strict), (Ok(0), Ok(0)));
    assert_eq!(memory.reads.borrow().len(), 3);
}

#[test]
fn capacity_and_short_copy_do_not_retry_or_publish_a_complete_prefix() {
    let mut memory = pointers(&[1, 2, 3]);
    let observed = observe_ledger(
        &memory,
        Array {
            data: 0x1000,
            count: 3,
        },
        &mut [0; 2],
    );
    assert_eq!(observed.required, 3);
    assert_eq!(observed.strict, Ok(3));
    assert_eq!(
        observed.nullable,
        Err("transport nullable ledger arena exhausted")
    );
    assert!(memory.reads.borrow().is_empty());
    memory.fail_at = Some(0x1000);
    memory.partial = true;
    let mut output = [0; 3];
    let observed = observe_ledger(
        &memory,
        Array {
            data: 0x1000,
            count: 3,
        },
        &mut output,
    );
    assert!(observed.nullable.is_err() && observed.strict.is_err());
    assert_eq!(output, [0; 3]);
    assert_eq!(memory.reads.borrow().len(), 1);
}

#[test]
fn original_null_failure_remains_strict_when_later_pointer_copy_fails() {
    let mut values = [1; 65];
    values[0] = 0;
    let mut memory = pointers(&values);
    memory.fail_at = Some(0x1000 + 64 * 8);
    let mut output = [0; 65];
    let observed = observe_ledger(
        &memory,
        Array {
            data: 0x1000,
            count: 65,
        },
        &mut output,
    );
    assert_eq!(
        observed.strict,
        Err("navy ledger mission unreadable or null")
    );
    assert!(observed.nullable.is_err());
    assert!(
        output[..64]
            .iter()
            .zip(&values[..64])
            .all(|(actual, original)| *actual == *original as usize)
    );
    assert_eq!(memory.reads.borrow().len(), 65);
}

#[test]
fn null_mission_and_backlink_skip_unreached_header_vtable_and_identity() {
    assert_eq!(
        receiver(0, 0, Err("unreached"), |_| panic!("unreached")),
        Ok(None)
    );
    assert_eq!(
        receiver(0, 0x2000, Ok(&header(0, 0)), |_| panic!("unreached")),
        Ok(None)
    );
}

#[test]
fn receiver_consumes_exact_original_class_handle_and_failure() {
    let task = 0x2000;
    let mission = task + ctask_force::NAVAL_MISSION;
    let header = header(task, cnaval_mission::VTABLE_RVA as u64);
    let full_handle = (19_u64 << 32) | 51;
    let calls = Cell::new(0);
    assert_eq!(
        receiver(0, mission, Ok(&header), |pointer| {
            assert_eq!(pointer, task);
            calls.set(calls.get() + 1);
            Ok((full_handle, 1))
        }),
        Ok(Some(Receiver {
            pointer: task,
            full_handle
        }))
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(
        receiver(0, mission, Ok(&header), |_| Err(
            "original registry refusal"
        )),
        Err("original registry refusal")
    );
    assert_eq!(
        receiver(0, mission, Ok(&header), |_| Ok((full_handle, 13))),
        Err("transport mission receiver is not original class1")
    );
}

#[test]
fn bad_embedding_vtable_and_short_header_do_not_demand_identity() {
    let task = 0x2000;
    let mission = task + ctask_force::NAVAL_MISSION;
    for bytes in [
        header(task, 0),
        header(task + 8, cnaval_mission::VTABLE_RVA as u64),
    ] {
        assert!(receiver(0, mission, Ok(&bytes), |_| panic!("unreached")).is_err());
    }
    assert!(receiver(0, mission, Ok(&[0; 15]), |_| panic!("unreached")).is_err());
}

const IMAGE: usize = 0x10_0000;
const TASK: usize = 0x2000;
const OTHER: usize = 0x2400;
const TABLE: usize = 0x8000;
const ENTRIES: usize = 0x9000;

fn unit(memory: &mut Fixture, pointer: usize, handle: u64, class: u32) {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&((IMAGE + ctask_force::VTABLE_RVA) as u64).to_le_bytes());
    bytes[8..12].copy_from_slice(&class.to_le_bytes());
    bytes[24..].copy_from_slice(&handle.to_le_bytes());
    memory.put(pointer, &bytes);
}

fn identity_fixture(kind: u32) -> (Fixture, u64, usize, usize) {
    use crate::{creference_registry as registry, reference_bucket as bucket};
    let handle = (71_u64 << 32) | u64::from(kind);
    let root = IMAGE
        + if kind < registry::LOW_ROOT_COUNT {
            registry::LOW_ROOTS_RVA + kind as usize * 8
        } else if kind < registry::MID_TYPE_LIMIT {
            registry::MID_ROOT_RVA
        } else {
            registry::HIGH_ROOT_RVA
        };
    let mut memory = Fixture::default();
    unit(&mut memory, TASK, handle, 1);
    unit(&mut memory, OTHER, (72_u64 << 32) | u64::from(kind), 1);
    memory.put(root, &(TABLE as u64).to_le_bytes());
    let mut header = [0; 17];
    header[..8].copy_from_slice(&(ENTRIES as u64).to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..16].copy_from_slice(&7_u32.to_le_bytes());
    header[16] = 5;
    memory.put(TABLE + registry::ENTRIES, &header);
    memory.put(ENTRIES, &[0; 14 * bucket::STRIDE]);
    memory.put(ENTRIES + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let index = ((hash >> 32) as u32 ^ hash as u32) & 7;
    let cell = ENTRIES + index as usize * bucket::STRIDE;
    let mut entry = [0; bucket::STRIDE];
    entry[bucket::DISTANCE] = 1;
    entry[8..16].copy_from_slice(&handle.to_le_bytes());
    entry[16..].copy_from_slice(&((TASK + ctask_force::SECONDARY_BASE) as u64).to_le_bytes());
    memory.put(cell, &entry);
    (memory, handle, root, cell)
}

#[test]
fn ship_sizing_borrows_the_native_nullable_receiver_and_authenticated_ship_header() {
    // Null missions and backlinks are skipped before ship-array demand.
    for kind in [
        ctask_force::DEFAULT_HANDLE_TYPE,
        ctask_force::OBSERVED_SHIP_SPACE_HANDLE_TYPE,
        crate::creference_registry::MID_TYPE_LIMIT,
    ] {
        let (mut memory, _, root, cell) = identity_fixture(kind);
        let mission = TASK + ctask_force::NAVAL_MISSION;
        memory.put(
            mission,
            &header(TASK, (IMAGE + cnaval_mission::VTABLE_RVA) as u64),
        );
        let mut ships = [0; 16];
        ships[..8].copy_from_slice(&0xa000_u64.to_le_bytes());
        ships[8..12].copy_from_slice(&5000_u32.to_le_bytes());
        ships[12..].copy_from_slice(&5000_u32.to_le_bytes());
        memory.put(TASK + ctask_force::SHIPS, &ships);
        assert_eq!(ship_demand(&memory, IMAGE, mission), Ok(5000));
        assert_eq!(memory.calls(0xa000), 0);

        memory.put(
            cell + crate::reference_bucket::OBJECT,
            &((TASK + 0x20) as u64).to_le_bytes(),
        );
        memory.reads.borrow_mut().clear();
        assert_eq!(
            ship_demand(&memory, IMAGE, mission),
            Err("counter unit registry identity mismatch")
        );
        assert_eq!(memory.calls(TASK + ctask_force::SHIPS), 0);

        memory.put(mission, &header(0, 0));
        memory.fail_at = Some(root);
        memory.reads.borrow_mut().clear();
        assert_eq!(ship_demand(&memory, IMAGE, mission), Ok(0));
        assert_eq!(*memory.reads.borrow(), [(mission, 16)]);
        memory.reads.borrow_mut().clear();
        assert_eq!(ship_demand(&memory, IMAGE, 0), Ok(0));
        assert!(memory.reads.borrow().is_empty());
    }
}

#[test]
fn authentic_full_registry_identity_is_acquired_once_for_repeated_receivers() {
    for kind in [61, 51, 4713] {
        let (memory, handle, root, cell) = identity_fixture(kind);
        let mut entries = [ReceiverIdentity::default(); 1];
        let mut slots = [PointerSlot::default(); 2];
        let mut reader = IdentityReader::new(&memory, IMAGE, &mut entries, &mut slots).unwrap();
        for _ in 0..3 {
            assert_eq!(reader.observe(TASK), Ok((handle, 1)));
        }
        assert_eq!((reader.len(), reader.required()), (1, 1));
        assert_eq!(
            (memory.calls(TASK), memory.calls(root), memory.calls(cell)),
            (1, 1, 1)
        );
    }
}

#[test]
fn identity_failures_and_failed_roots_cannot_be_healed_by_repeat_or_other_receiver() {
    let (mut memory, _, root, _) = identity_fixture(61);
    memory.fail_at = Some(root);
    memory.fail_once = true;
    let mut entries = [ReceiverIdentity::default(); 2];
    let mut slots = [PointerSlot::default(); 4];
    let mut reader = IdentityReader::new(&memory, IMAGE, &mut entries, &mut slots).unwrap();
    let first = reader.observe(TASK);
    assert_eq!(first, Err("reference root unreadable"));
    assert_eq!(reader.observe(TASK), first);
    assert_eq!(reader.observe(OTHER), first);
    assert_eq!(
        (memory.calls(TASK), memory.calls(OTHER), memory.calls(root)),
        (1, 1, 1)
    );
    assert_eq!((reader.len(), reader.required()), (2, 2));
}

#[test]
fn wrong_registry_secondary_class_or_handle_is_never_a_successful_receiver() {
    for form in 0..3 {
        let (mut memory, handle, _, cell) = identity_fixture(61);
        match form {
            0 => memory.put(cell + 16, &(TASK as u64).to_le_bytes()),
            1 => unit(&mut memory, TASK, handle, 13),
            _ => unit(&mut memory, TASK, 61, 1),
        }
        let mut entries = [ReceiverIdentity::default(); 1];
        let mut slots = [PointerSlot::default(); 2];
        let mut reader = IdentityReader::new(&memory, IMAGE, &mut entries, &mut slots).unwrap();
        let first = reader.observe(TASK);
        assert!(first.is_err());
        assert_eq!(reader.observe(TASK), first);
        assert_eq!(memory.calls(TASK), 1);
    }
}

#[test]
fn exhausted_identity_capacity_preserves_known_facts_and_never_probes_unknowns() {
    let (memory, handle, _, _) = identity_fixture(61);
    let mut entries = [ReceiverIdentity::default(); 1];
    let mut slots = [PointerSlot::default(); 2];
    let mut reader = IdentityReader::new(&memory, IMAGE, &mut entries, &mut slots).unwrap();
    assert_eq!(reader.observe(TASK), Ok((handle, 1)));
    let before = memory.reads.borrow().len();
    for pointer in [OTHER, OTHER, OTHER + 0x1000] {
        assert_eq!(
            reader.observe(pointer),
            Err("transport receiver identity arena exhausted")
        );
    }
    assert_eq!(reader.observe(TASK), Ok((handle, 1)));
    assert_eq!(memory.reads.borrow().len(), before);
    assert_eq!((reader.len(), reader.required()), (1, 2));
}

#[test]
fn original_native_skips_and_signed_combat_presence_control_risk_demand() {
    assert_eq!(
        risk_demand(
            Ok(false),
            Err("unreached"),
            Err("unreached"),
            Err("unreached")
        ),
        Ok(false)
    );
    assert_eq!(
        risk_demand(Ok(true), Ok(false), Err("unreached"), Err("unreached")),
        Ok(false)
    );
    assert_eq!(
        risk_demand(Ok(true), Ok(true), Ok(0), Err("unreached")),
        Ok(false)
    );
    for (count, expected) in [(i32::MIN, true), (-1, true), (0, true), (1, false)] {
        assert_eq!(
            risk_demand(Ok(true), Ok(true), Ok(255), Ok(count)),
            Ok(expected)
        );
    }
    assert_eq!(
        risk_demand(Ok(true), Ok(true), Ok(1), Err("original count failure")),
        Err("original count failure")
    );
}

#[test]
fn admitted_pointer_index_preserves_5000_unordered_keys_repeats_and_null() {
    let count = 5000;
    let mut slots = vec![PointerSlot::default(); index_capacity(count).unwrap()];
    {
        let mut index = PointerIndex::new(&mut slots, count).unwrap();
        for ordinal in (0..count).rev() {
            index.insert(ordinal * 0x100, ordinal).unwrap();
        }
        for ordinal in 0..count {
            assert_eq!(index.find(ordinal * 0x100), Some(ordinal));
            index.insert(ordinal * 0x100, ordinal).unwrap();
        }
        assert_eq!(index.find(count * 0x100), None);
        assert!(index.insert(count * 0x100, 0).is_err());
        assert!(index.insert(0, 1).is_err());
    }
    assert!(slots.iter().all(|slot| *slot == PointerSlot::default()));
}

#[test]
fn worker_pointer_index_grows_from_zero_and_preserves_all_original_entries() {
    let mut slots = Vec::new();
    {
        let mut index = PointerIndex::new_growing(&mut slots, 0).unwrap();
        for ordinal in 0..5000 {
            index.insert(ordinal * 0x100, ordinal).unwrap();
        }
        for ordinal in (0..5000).rev() {
            assert_eq!(index.find(ordinal * 0x100), Some(ordinal));
            index.insert(ordinal * 0x100, ordinal).unwrap();
        }
        assert!(index.insert(0, 5000).is_err());
        assert_eq!(index.find(0), Some(0));
    }
    assert!(slots.iter().all(|slot| *slot == PointerSlot::default()));
}

#[test]
fn zero_capacity_pointer_index_is_empty_and_refuses_insert_without_storage() {
    assert_eq!(index_capacity(0), Some(0));
    let mut slots = [];
    let mut index = PointerIndex::new(&mut slots, 0).unwrap();
    assert_eq!(index.find(0), None);
    assert_eq!(index.find(usize::MAX), None);
    assert_eq!(
        index.insert(0, 0),
        Err("transport pointer index entries exhausted")
    );
}

#[test]
fn invalid_pointer_index_capacity_refuses_without_native_reads() {
    assert!(index_capacity(usize::MAX).is_none());
    let (memory, _, _, _) = identity_fixture(61);
    let mut entries = [ReceiverIdentity::default(); 2];
    assert!(
        IdentityReader::new(
            &memory,
            IMAGE,
            &mut entries,
            &mut [PointerSlot::default(); 2]
        )
        .is_err()
    );
    assert!(PointerIndex::new(&mut [PointerSlot::default(); 3], 1).is_err());
    assert!(memory.reads.borrow().is_empty());
}
