use super::*;
use engine::{cnaval_mission, creference_registry as registry, cunit, reference_bucket as bucket};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

const IMAGE: usize = 0x10_0000;
const TASK: usize = 0x2000;
const TABLE: usize = 0x8000;
const ENTRIES: usize = 0x9000;
const MISSION: usize = TASK + ctask_force::NAVAL_MISSION;
const HANDLE: u64 = (71_u64 << 32) | 4713;

#[derive(Default)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fail_once_at: Option<usize>,
    failed: Cell<bool>,
}

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fail_once_at == Some(address) && !self.failed.replace(true) {
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

fn fixture() -> Fixture {
    let mut memory = Fixture::default();
    let mut header = [0; 32];
    header[..8].copy_from_slice(&((IMAGE + ctask_force::VTABLE_RVA) as u64).to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[24..].copy_from_slice(&HANDLE.to_le_bytes());
    memory.put(TASK, &header);
    let mut mission = [0; 16];
    mission[..8].copy_from_slice(&((IMAGE + cnaval_mission::VTABLE_RVA) as u64).to_le_bytes());
    mission[8..].copy_from_slice(&(TASK as u64).to_le_bytes());
    memory.put(MISSION, &mission);
    memory.put(MISSION + cnaval_mission::IS_IN_REGIONS, &[255]);
    memory.put(
        TASK + cunit::RECEIVED_SUPPLY_RATIO,
        &(-35_001_i64).to_le_bytes(),
    );
    memory.put(
        IMAGE + registry::HIGH_ROOT_RVA,
        &(TABLE as u64).to_le_bytes(),
    );
    let mut root = [0; 17];
    root[..8].copy_from_slice(&(ENTRIES as u64).to_le_bytes());
    root[8..12].copy_from_slice(&1_u32.to_le_bytes());
    root[12..16].copy_from_slice(&7_u32.to_le_bytes());
    root[16] = 5;
    memory.put(TABLE + registry::ENTRIES, &root);
    memory.put(ENTRIES, &[0; 14 * bucket::STRIDE]);
    memory.put(ENTRIES + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
    let hash = HANDLE
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let index = ((hash >> 32) as u32 ^ hash as u32) & 7;
    let mut entry = [0; bucket::STRIDE];
    entry[bucket::DISTANCE] = 1;
    entry[8..16].copy_from_slice(&HANDLE.to_le_bytes());
    entry[16..].copy_from_slice(&((TASK + ctask_force::SECONDARY_BASE) as u64).to_le_bytes());
    memory.put(ENTRIES + index as usize * bucket::STRIDE, &entry);
    memory
}

fn admit(captured: &mut Captured, memory: &Fixture) {
    let required = {
        let mut observer = captured.observer(memory, IMAGE).unwrap();
        assert_eq!(observer.observe(MISSION, None), Ok(HANDLE));
        observer.required()
    };
    captured.finish_identities(required);
}

#[test]
fn repeated_mission_borrows_original_header_and_full_identity_exactly_once() {
    let memory = fixture();
    let original_header = read::<[u8; 16]>(&memory, MISSION).unwrap();
    let mut captured = Captured::new(2);
    let required = {
        let mut observer = captured.observer(&memory, IMAGE).unwrap();
        assert_eq!(
            observer.observe(MISSION, Some(Ok(original_header))),
            Ok(HANDLE)
        );
        assert_eq!(observer.observe(MISSION, None), Ok(HANDLE));
        assert_eq!(
            observer.observe(MISSION, Some(Err("later header refusal"))),
            Ok(HANDLE)
        );
        observer.required()
    };
    captured.finish_identities(required);
    assert_eq!(memory.calls(MISSION), 1);
    assert_eq!(memory.calls(TASK), 1);
    assert_eq!(memory.calls(IMAGE + registry::HIGH_ROOT_RVA), 1);
    assert_eq!(
        (
            captured.missions.len,
            captured.controls.len,
            captured.identities.required
        ),
        (1, 1, 1)
    );
    assert_eq!(captured.controls.first[0].pointer, TASK);
    assert_eq!(captured.controls.first[0].packet.task_force, HANDLE);
}

#[test]
fn supplied_original_header_failure_never_reprobes_readable_memory_or_unit() {
    let memory = fixture();
    let mut captured = Captured::new(2);
    let required = {
        let mut observer = captured.observer(&memory, IMAGE).unwrap();
        assert_eq!(
            observer.observe(MISSION, Some(Err("original header short copy"))),
            Err("original header short copy")
        );
        assert_eq!(
            observer.observe(MISSION, None),
            Err("original header short copy")
        );
        observer.required()
    };
    captured.finish_identities(required);
    assert!(memory.reads.borrow().is_empty());
    assert_eq!(captured.missions.len, 1);
    assert_eq!(
        (captured.controls.len, captured.identities.required),
        (0, 0)
    );
}

#[test]
fn failed_once_original_header_and_registry_results_are_not_healed_on_repeat() {
    for (address, reason) in [
        (MISSION, "transport original mission header unreadable"),
        (IMAGE + registry::HIGH_ROOT_RVA, "reference root unreadable"),
    ] {
        let mut memory = fixture();
        memory.fail_once_at = Some(address);
        let mut captured = Captured::new(2);
        let required = {
            let mut observer = captured.observer(&memory, IMAGE).unwrap();
            assert_eq!(observer.observe(MISSION, None), Err(reason));
            assert_eq!(observer.observe(MISSION, None), Err(reason));
            observer.required()
        };
        captured.finish_identities(required);
        assert_eq!(memory.calls(address), 1);
        assert_eq!(memory.calls(MISSION), 1);
        assert_eq!(captured.controls.len, 0);
    }
}

#[test]
fn original_failed_counter_prefix_keeps_missing_scalars_and_never_retries() {
    let memory = fixture();
    let mut captured = Captured::new(2);
    admit(&mut captured, &memory);
    let before = memory.reads.borrow().len();
    assert_eq!(
        captured.supply_counter(
            &memory,
            TASK,
            HANDLE,
            Some(Err("original counter prefix refused"))
        ),
        Ok(true)
    );
    let control = &captured.controls.first[0];
    assert!(control.counter_attempted);
    assert_eq!(control.packet.in_regions, None);
    assert_eq!(control.packet.received_supply_ratio, None);
    assert_eq!(
        captured.supply_counter(&memory, TASK, HANDLE, None),
        Err("transport original counter prefix attempted twice")
    );
    assert_eq!(memory.reads.borrow().len(), before);
}

#[test]
fn original_unattempted_counter_capacity_acquires_each_scalar_once() {
    let memory = fixture();
    let mut captured = Captured::new(2);
    admit(&mut captured, &memory);
    let before = memory.reads.borrow().len();
    assert_eq!(
        captured.supply_counter(&memory, TASK, HANDLE, None),
        Ok(true)
    );
    let control = &captured.controls.first[0];
    assert!(control.counter_attempted);
    assert_eq!(control.packet.in_regions, Some(255));
    assert_eq!(control.packet.received_supply_ratio, Some(-35_001));
    assert_eq!(memory.calls(MISSION + cnaval_mission::IS_IN_REGIONS), 1);
    assert_eq!(memory.calls(TASK + cunit::RECEIVED_SUPPLY_RATIO), 1);
    assert_eq!(memory.reads.borrow().len(), before + 2);
    assert_eq!(
        captured.supply_counter(&memory, TASK, HANDLE, None),
        Err("transport original counter prefix attempted twice")
    );
    assert_eq!(memory.reads.borrow().len(), before + 2);
}

#[test]
fn original_successful_counter_values_are_borrowed_instead_of_rereading_changed_memory() {
    let memory = fixture();
    let mut captured = Captured::new(2);
    admit(&mut captured, &memory);
    let before = memory.reads.borrow().len();
    let original = ctask_force::CounterRow {
        id: HANDLE,
        is_in_regions: 17,
        received_supply_ratio: -90_001,
        ..Default::default()
    };
    assert_eq!(
        captured.supply_counter(&memory, TASK, HANDLE, Some(Ok(original))),
        Ok(true)
    );
    assert_eq!(captured.controls.first[0].packet.in_regions, Some(17));
    assert_eq!(
        captured.controls.first[0].packet.received_supply_ratio,
        Some(-90_001)
    );
    assert_eq!(memory.reads.borrow().len(), before);
}

#[test]
fn original_mission_owner_success_and_failure_are_borrowed_without_retry() {
    for original in [Ok(123_456), Err("original mission owner unreadable")] {
        let mut memory = fixture();
        let at = MISSION + engine::naval_mission_inputs::MISSION_OWNER;
        memory.put(at, &99_u32.to_le_bytes());
        let header = read::<[u8; 16]>(&memory, MISSION).unwrap();
        let mut captured = Captured::new(2);
        let required = {
            let mut observer = captured.observer(&memory, IMAGE).unwrap();
            assert_eq!(
                observer.observe_transport(MISSION, Some(Ok(header)), Some(original)),
                Ok(HANDLE)
            );
            assert_eq!(
                observer.observe_transport(MISSION, None, Some(Ok(99))),
                Ok(HANDLE)
            );
            observer.required()
        };
        captured.finish_identities(required);
        assert_eq!(
            captured.controls.first[0].packet.mission_owner,
            original.ok()
        );
        assert_eq!(memory.calls(at), 0);
        assert_eq!(memory.calls(MISSION), 1);
    }
}

#[test]
fn unattempted_mission_owner_is_read_once_and_failed_result_survives_repeat() {
    for fail in [false, true] {
        let mut memory = fixture();
        let at = MISSION + engine::naval_mission_inputs::MISSION_OWNER;
        memory.put(at, &123_456_u32.to_le_bytes());
        if fail {
            memory.fail_once_at = Some(at);
        }
        let mut captured = Captured::new(2);
        let required = {
            let mut observer = captured.observer(&memory, IMAGE).unwrap();
            assert_eq!(observer.observe_transport(MISSION, None, None), Ok(HANDLE));
            assert_eq!(
                observer.observe_transport(MISSION, None, Some(Ok(99))),
                Ok(HANDLE)
            );
            observer.required()
        };
        captured.finish_identities(required);
        assert_eq!(
            captured.controls.first[0].packet.mission_owner,
            if fail { None } else { Some(123_456) }
        );
        assert_eq!(memory.calls(at), 1);
    }
}

#[test]
fn retire_clears_native_pointer_certificates_and_reset_retires_the_old_pass() {
    let memory = fixture();
    let mut captured = Captured::new(2);
    admit(&mut captured, &memory);
    captured
        .supply_counter(&memory, TASK, HANDLE, None)
        .unwrap();
    assert!(
        captured
            .identities
            .first
            .iter()
            .any(|identity| identity.pointer == TASK)
    );
    assert!(
        captured
            .missions
            .first
            .iter()
            .any(|mission| mission.result.is_some())
    );
    let packet = captured.controls.first[0].packet;
    let before = memory.reads.borrow().len();
    captured.retire();
    assert!(
        captured
            .controls
            .first
            .iter()
            .all(|control| control.pointer == 0)
    );
    assert!(
        captured
            .identities
            .first
            .iter()
            .all(|identity| *identity == native::ReceiverIdentity::default())
    );
    assert!(
        captured
            .missions
            .first
            .iter()
            .all(|mission| mission.result.is_none())
    );
    assert!(
        captured
            .identity_slots
            .iter()
            .chain(&captured.mission_slots)
            .all(|slot| *slot == native::PointerSlot::default())
    );
    assert_eq!(captured.controls.first[0].packet, packet);
    assert_eq!(memory.reads.borrow().len(), before);
    captured.reset();
    assert_eq!(
        (
            captured.controls.len,
            captured.missions.len,
            captured.identities.len
        ),
        (0, 0, 0)
    );
    assert_eq!(
        (
            captured.controls.required,
            captured.missions.required,
            captured.identities.required
        ),
        (0, 0, 0)
    );
}

#[test]
fn zero_sized_vacant_receiver_holder_has_no_allocated_storage_or_native_reads() {
    let memory = fixture();
    let mut captured = Captured::new(0);
    assert_eq!(captured.allocated_bytes(), 0);
    assert_eq!(captured.identity_slots.capacity(), 0);
    assert_eq!(captured.mission_slots.capacity(), 0);
    assert!(captured.observer(&memory, IMAGE).is_err());
    assert!(memory.reads.borrow().is_empty());
    captured.reset();
    assert_eq!(captured.allocated_bytes(), 0);
}

#[test]
fn worker_receiver_identity_and_mission_memos_grow_and_keep_original_failures_once() {
    let memory = fixture();
    let mut captured = Captured::new(0);
    let mut observer = captured.observer_growing(&memory, IMAGE).unwrap();
    assert_eq!(observer.observe(MISSION, None), Ok(HANDLE));
    // Distinct failed certificates still require complete same-pass memo room.
    // Repeated occurrences reuse their first failure without a second read.
    for index in 0..5000 {
        let pointer = 0x10_0000 + index * 0x1000;
        let mission = pointer + ctask_force::NAVAL_MISSION;
        let mut header = [0; 16];
        header[..8].copy_from_slice(&((IMAGE + cnaval_mission::VTABLE_RVA) as u64).to_le_bytes());
        header[8..].copy_from_slice(&(pointer as u64).to_le_bytes());
        let first = observer.observe(mission, Some(Ok(header)));
        assert!(first.is_err());
        let reads = memory.reads.borrow().len();
        assert_eq!(
            observer.observe(mission, Some(Err("later alias refused"))),
            first
        );
        assert_eq!(memory.reads.borrow().len(), reads);
    }
    assert_eq!(observer.required(), 5001);
    drop(observer);
    assert!(captured.identities.first.len() >= 5001);
    assert_eq!(captured.missions.len, 5001);
    assert_eq!(captured.controls.len, 1);
}
