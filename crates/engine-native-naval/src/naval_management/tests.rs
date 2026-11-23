use super::*;
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Default)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        for (offset, &byte) in bytes.iter().enumerate() {
            self.bytes.insert(at + offset, byte);
        }
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn integer(&mut self, at: usize, value: i32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, data as u64);
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        for (offset, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + offset)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

#[test]
fn naval_management_composition_retains_every_preference_and_native_order() {
    let mut memory = Fixture::default();
    const OBJECT: usize = 0x1000;
    const DATA: usize = 0x2000;
    memory.word(OBJECT, ENTRIES_VTABLE_RVA.0 as u64);
    memory.array(OBJECT + COMPOSITION_ENTRIES, DATA, 3);
    let values = [(19, 7, -1, 0), (19, 7, 42, -5), (3, -2, i32::MAX, i32::MIN)];
    for (index, (sub_unit, role, definition, amount)) in values.into_iter().enumerate() {
        for (word, value) in [sub_unit, role, definition, amount].into_iter().enumerate() {
            memory.integer(DATA + index * COMPOSITION_ENTRY_BYTES + word * 4, value);
        }
    }
    let mut entries = Vec::new();
    composition(&memory, 0, OBJECT, &mut entries).unwrap();
    assert_eq!(
        entries,
        values.map(|(sub_unit, role, definition, amount)| CompositionEntry {
            sub_unit,
            role,
            definition,
            amount
        })
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at >= DATA)
            .count(),
        3
    );
    memory.bytes.remove(&(DATA + COMPOSITION_ENTRY_BYTES + 12));
    assert!(composition(&memory, 0, OBJECT, &mut entries).is_err());
}

#[test]
fn naval_management_composition_grows_to_live_demand_without_a_chosen_limit() {
    const OBJECT: usize = 0x1000;
    const DATA: usize = 0x2000;
    let mut memory = Fixture::default();
    memory.word(OBJECT, ENTRIES_VTABLE_RVA.0 as u64);
    memory.array(OBJECT + COMPOSITION_ENTRIES, DATA, 5_000);
    for index in 0..5_000 {
        memory.put(
            DATA + index * COMPOSITION_ENTRY_BYTES,
            &[0xff; COMPOSITION_ENTRY_BYTES],
        );
    }
    let mut entries = Vec::new();
    composition(&memory, 0, OBJECT, &mut entries).unwrap();
    assert_eq!(entries.len(), 5_000);
    assert!(entries.iter().all(|entry| entry.amount == -1));
}

#[test]
fn naval_management_repair_queue_preserves_typed_handles_repeats_and_empty() {
    const OBJECT: usize = 0x1000;
    const DATA: usize = 0x2000;
    let mut memory = Fixture::default();
    memory.word(OBJECT, crate::cnaval_base::VTABLE_RVA.0 as u64);
    memory.integer(OBJECT + BASE_REPAIR_PRIORITY, -9);
    memory.integer(OBJECT + BASE_REPAIR_DOCKYARDS, 17);
    memory.array(OBJECT + BASE_REPAIR_QUEUE, DATA, 3);
    let handles = [u64::MAX, 51 | (7 << 32), u64::MAX];
    for (index, handle) in handles.into_iter().enumerate() {
        memory.word(DATA + index * 8, handle);
    }
    let mut queue = Vec::new();
    assert_eq!(
        base_settings(&memory, 0, OBJECT, &mut queue).unwrap(),
        BaseSettings {
            priority: -9,
            dockyards: 17
        }
    );
    assert_eq!(queue, handles);
    memory.array(OBJECT + BASE_REPAIR_QUEUE, 0, 0);
    base_settings(&memory, 0, OBJECT, &mut queue).unwrap();
    assert!(queue.is_empty());
    memory.array(OBJECT + BASE_REPAIR_QUEUE, 0, 1);
    assert!(base_settings(&memory, 0, OBJECT, &mut queue).is_err());
}

#[test]
fn naval_management_task_controls_keep_independent_refit_designs_and_target() {
    const TASK: usize = 0x1000;
    const STATE: usize = 0x8000;
    const PROVINCES: usize = 0x2000;
    const TARGET: usize = 0x3000;
    const VARIANT: usize = 0x4000;
    let mut memory = Fixture::default();
    memory.array(STATE + engine_native::cgame_state::PROVINCES, PROVINCES, 3);
    memory.word(PROVINCES + 2 * 8, TARGET as u64);
    memory.word(TARGET, engine_native::cprovince::VTABLE_RVA.0 as u64);
    memory.put(TARGET + engine_native::cprovince::ID, &2_u32.to_le_bytes());
    memory.word(TASK + engine_native::ctask_force::REPAIR_TARGET, TARGET as u64);
    let rejoin = u64::from(engine_native::ctask_force::DEFAULT_HANDLE_TYPE) | (8_u64 << 32);
    memory.word(TASK + MERGE_AFTER_REPAIR, rejoin);
    memory.integer(TASK + REPAIR_MODE, -7);
    memory.put(TASK + REPAIR_SPLIT, &[3]);
    memory.put(TASK + AUTO_REINFORCEMENT, &[0]);
    memory.word(TASK + REFIT_VARIANT, 0);
    memory.word(TASK + REFIT_AFTER_REPAIR, VARIANT as u64);
    memory.word(VARIANT, engine_native::cequipment_variant::VTABLE_RVA.0 as u64);
    let design = u64::from(engine_native::cequipment_variant::REFERENCE_TYPE) | (u64::from(u32::MAX) << 32);
    memory.word(VARIANT + engine_native::cequipment_variant::TYPE, design);
    memory.put(TASK + COMPOSITION_TOKEN, &0xfedc_ba98_u32.to_le_bytes());
    let value = task_settings(&memory, 0, STATE, TASK).unwrap();
    assert_eq!(
        value,
        TaskSettings {
            repair_mode: -7,
            repair_split: 3,
            repair_target: 2,
            auto_reinforcement: 0,
            refit_variant: 0,
            merge_after_repair: rejoin,
            refit_after_repair: design,
            composition_token: 0xfedc_ba98
        }
    );
    memory.word(PROVINCES + 2 * 8, 0);
    assert!(task_settings(&memory, 0, STATE, TASK).is_err());
}

#[test]
fn naval_management_children_keep_order_and_unresolved_full_references() {
    let mut memory = Fixture::default();
    memory.array(0x1000, 0x2000, 3);
    let expected = [u64::MAX, 61 | (9 << 32), u64::MAX];
    for (index, value) in expected.iter().enumerate() {
        memory.word(0x2000 + index * 8, *value);
    }
    let mut children = Vec::new();
    repair_children(&memory, 0x1000, &mut children).unwrap();
    assert_eq!(children, expected);
    memory.array(0x1000, 0, 0);
    repair_children(&memory, 0x1000, &mut children).unwrap();
    assert!(children.is_empty());
    memory.array(0x1000, 0, 3);
    assert!(repair_children(&memory, 0x1000, &mut children).is_err());
}

#[test]
fn naval_management_budgets_and_refit_absence_keep_native_signed_values() {
    let mut memory = Fixture::default();
    memory.integer(0x1000 + MAX_FACTORIES_FOR_REPAIR, i32::MIN);
    memory.integer(0x2000 + MAX_REPAIR_DOCKYARDS, -1);
    memory.integer(0x2000 + USED_REPAIR_DOCKYARDS, i32::MAX);
    assert_eq!(max_factories_for_repair(&memory, 0x1000).unwrap(), i32::MIN);
    assert_eq!(repair_dockyards(&memory, 0x2000).unwrap(), (-1, i32::MAX));
    let absent = u64::MAX;
    memory.word(engine_native_parts::creference_registry::INVALID_REFERENCE_RVA.0, absent);
    memory.word(0x3000 + SHIP_REFIT_LINE, absent);
    assert_eq!(ship_refit_line(&memory, 0, 0x3000).unwrap(), 0);
    memory.word(0x3000 + SHIP_REFIT_LINE, 71 | (19 << 32));
    assert_eq!(
        ship_refit_line(&memory, 0, 0x3000).unwrap(),
        71 | (19 << 32)
    );
}
