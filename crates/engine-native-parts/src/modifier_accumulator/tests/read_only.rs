use super::*;

#[test]
fn existing_lookup_does_not_read_or_mutate_unobserved_success_or_absent_key() {
    let memory = SnapshotMemory::new(2, None);
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 2,
        entries: 4,
    });
    let address = 0x1000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    assert_eq!(arena.existing_at(address, 1), None);
    assert_eq!(memory.calls.get(), 0);
    arena.capture_at(&memory, address, 1).unwrap();
    let calls = memory.calls.get();
    let required = arena.required_capacity();
    let allocated = arena.allocated_bytes();
    let state = (
        arena.nodes.len(),
        arena.entries.len(),
        arena.index.clone(),
        arena.generation,
    );
    assert_eq!(
        arena.existing_at(address, 1),
        Some(Ok(Value {
            raw: 0,
            present: true
        }))
    );
    assert_eq!(
        arena.existing_at(address, 2),
        Some(Ok(Value {
            raw: -15,
            present: true
        }))
    );
    assert_eq!(arena.existing_at(address, 3), Some(Ok(Value::default())));
    assert_eq!(arena.existing_at(address + 32, 1), None);
    assert_eq!(memory.calls.get(), calls);
    assert_eq!(arena.required_capacity(), required);
    assert_eq!(arena.allocated_bytes(), allocated);
    assert_eq!(
        (
            arena.nodes.len(),
            arena.entries.len(),
            arena.index.clone(),
            arena.generation
        ),
        state
    );
    arena.clear();
    assert_eq!(arena.existing_at(address, 1), None);
    assert_eq!(memory.calls.get(), calls);
}

#[test]
fn read_only_lookup_preserves_failure_without_healing_it_or_charging_demand() {
    let mut memory = SnapshotMemory::new(2, Some(0x1610));
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 2,
    });
    let address = 0x1000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    let failed = Err("modifier accumulator entry unreadable");
    assert_eq!(
        arena.capture_at(&memory, address, 1).unwrap_err(),
        "modifier accumulator entry unreadable"
    );
    let calls = memory.calls.get();
    let required = arena.required_capacity();
    memory.failure = None;
    assert_eq!(arena.existing_at(address, 1), Some(failed));
    assert_eq!(arena.existing_at(address, 2), Some(failed));
    assert_eq!(memory.calls.get(), calls);
    assert_eq!(arena.required_capacity(), required);
}
