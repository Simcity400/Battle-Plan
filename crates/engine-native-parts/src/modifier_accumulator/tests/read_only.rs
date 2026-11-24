use super::*;

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
