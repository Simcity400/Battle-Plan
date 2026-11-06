use super::*;
use crate::production::variant_scale_tests::{Space, space};

fn prepare_fixture(space: &Space, registry: &[native::RegistryRow]) {
    let mut variants = Variants::vacant();
    variants.capture_state_carried(
        &space.memory(),
        space.base,
        Sources {
            rows: registry,
            outcome: Outcome::Rows {
                len: registry.len(),
            },
        },
        7,
        true,
        false,
    );
    assert_eq!(
        variants.definitions.outcome,
        Outcome::Rows {
            len: registry.len()
        }
    );
    begin_full();
    begin_read();
    prepare(registry, &variants);
    assert!(baseline_admitted());
}

#[test]
fn retained_values_follow_prepare_real_drain_failure_teardown_and_reattachment() {
    let mut space = space(1, 1);
    let mut registry = Vec::new();
    assert_eq!(
        native::walk_registry(&space.memory(), space.base, space.game_state, &mut registry),
        Outcome::Rows { len: 1 }
    );
    let pointer = registry[0].pointer();
    let start = space.bytes.as_ptr() as usize;
    let origin = pointer + native::ORIGIN_LINK - start;
    space.bytes[origin..origin + 4].copy_from_slice(&0_i32.to_le_bytes());
    let clock = space.game_state + engine_native::cgame_state::CURRENT_HOUR - start;
    space.bytes[clock..clock + 4].copy_from_slice(&7_i32.to_le_bytes());
    let stat = pointer + native::MAP_RANGE_STAT - start;
    let original =
        engine_native::read::<i64>(&space.memory(), pointer + native::MAP_RANGE_STAT).unwrap();
    let empty = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let hooks = unsafe { Hooks::new(&empty, space.base) }.unwrap();
    assert!(hooks.hooks.is_empty());
    boundary(true);
    ENABLED.store(true, Ordering::Release);
    prepare_fixture(&space, &registry);
    let mut output = Vec::new();
    let mut shared = Shared::default();
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert!(output.is_empty());
    assert_eq!(shared.len(), 1);
    let first = Arc::clone(&shared.entries[0].value);
    assert_eq!(
        retained(EPOCH.load(Ordering::Acquire), space.base).state_count(),
        1
    );
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert!(Arc::ptr_eq(&shared.entries[0].value, &first));

    space.bytes[stat..stat + 8].copy_from_slice(&original.wrapping_add(1).to_le_bytes());
    observe(&space.memory(), space.base, pointer, false, false);
    {
        let mut store = STORE.get().unwrap().lock().unwrap();
        let event = *store.batch.events.last().unwrap();
        store.batch.events.push(event);
    }
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Err("variant occurrences are not in native observation order")
    );
    assert!(RECOVER.load(Ordering::Acquire));
    assert!(!baseline_admitted());
    assert_eq!(shared.len(), 1);
    assert!(!Arc::ptr_eq(&shared.entries[0].value, &first));
    assert_ne!(shared.entries[0].value.value, first.value);
    assert_eq!(
        retained(EPOCH.load(Ordering::Acquire), space.base).state_count(),
        0
    );
    space.bytes[stat..stat + 8].copy_from_slice(&original.to_le_bytes());
    prepare_fixture(&space, &registry);
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert_eq!(shared.entries[0].value.value, first.value);
    assert!(!Arc::ptr_eq(&shared.entries[0].value, &first));
    let recovered = Arc::clone(&shared.entries[0].value);
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert!(Arc::ptr_eq(&shared.entries[0].value, &recovered));

    drop(hooks);
    assert!(!enabled());
    assert_eq!(
        retained(EPOCH.load(Ordering::Acquire), space.base).state_count(),
        0
    );
    let hooks = unsafe { Hooks::new(&empty, space.base) }.unwrap();
    assert!(hooks.hooks.is_empty());
    ENABLED.store(true, Ordering::Release);
    prepare_fixture(&space, &registry);
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert_eq!(shared.len(), 1);
    assert_eq!(shared.entries[0].value.value, first.value);
    assert!(!Arc::ptr_eq(&shared.entries[0].value, &first));
    let attached = Arc::clone(&shared.entries[0].value);
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert!(Arc::ptr_eq(&shared.entries[0].value, &attached));
    drop(hooks);
    assert!(!enabled());
    assert_eq!(
        retained(EPOCH.load(Ordering::Acquire), space.base).state_count(),
        0
    );
}

#[test]
fn failed_drains_and_terminal_repairs_discard_pre_shrink_states_before_recovery_and_growth() {
    fn put(space: &mut Space, address: usize, bytes: &[u8]) {
        let at = address - space.bytes.as_ptr() as usize;
        space.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn source_shape(space: &mut Space, pointer: usize, capacity: u32, count: u32) {
        put(
            space,
            pointer + native::BONUS_SOURCES + 8,
            &capacity.to_le_bytes(),
        );
        put(
            space,
            pointer + native::BONUS_SOURCES + 12,
            &count.to_le_bytes(),
        );
    }
    let mut space = space(1, 1);
    let mut registry = Vec::new();
    assert_eq!(
        native::walk_registry(&space.memory(), space.base, space.game_state, &mut registry),
        Outcome::Rows { len: 1 }
    );
    let pointer = registry[0].pointer();
    put(
        &mut space,
        pointer + native::ORIGIN_LINK,
        &0_i32.to_le_bytes(),
    );
    let clock = space.game_state + engine_native::cgame_state::CURRENT_HOUR;
    put(&mut space, clock, &7_i32.to_le_bytes());
    // The fixture's unused tail holds three distinct, empty synthetic sources and their roster.
    let stride = native::BONUS_IDEA.next_multiple_of(16);
    let tail = space.bytes.len() - (3 * stride + 24);
    assert!(space.bytes[tail..].iter().all(|byte| *byte == 0));
    let sources = space.bytes.as_ptr() as usize + tail;
    let roster = sources + 3 * stride;
    for ordinal in 0..3 {
        let source = sources + ordinal * stride;
        let table = (space.base + native::SPECIFIC_BONUS_VTABLE_RVA.0) as u64;
        put(&mut space, source, &table.to_le_bytes());
        put(
            &mut space,
            roster + ordinal * 8,
            &(source as u64).to_le_bytes(),
        );
    }
    put(
        &mut space,
        pointer + native::BONUS_SOURCES,
        &(roster as u64).to_le_bytes(),
    );
    source_shape(&mut space, pointer, 3, 3);
    let empty = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let hooks = unsafe { Hooks::new(&empty, space.base) }.unwrap();
    assert!(hooks.hooks.is_empty());
    boundary(true);
    ENABLED.store(true, Ordering::Release);
    prepare_fixture(&space, &registry);
    let epoch = EPOCH.load(Ordering::Acquire);
    let mut output = Vec::new();
    let mut shared = Shared::default();
    let mut previous = Vec::new();
    for count in 0..=3 {
        source_shape(&mut space, pointer, 3, count);
        observe(&space.memory(), space.base, pointer, false, false);
        assert_eq!(
            drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
            Ok(0)
        );
        assert!(output.is_empty());
        assert_eq!(shared.len(), 1);
        assert_eq!(
            shared.entries[0]
                .value
                .definition
                .design
                .bonus_sources
                .len(),
            count as usize
        );
        previous.push(Arc::clone(&shared.entries[0].value));
        assert_eq!(
            retained(epoch, space.base).state_count(),
            count as usize + 1
        );
    }
    source_shape(&mut space, pointer, 0, 0);
    let stat =
        engine_native::read::<i64>(&space.memory(), pointer + native::MAP_RANGE_STAT).unwrap();
    put(
        &mut space,
        pointer + native::MAP_RANGE_STAT,
        &stat.wrapping_add(1).to_le_bytes(),
    );
    observe(&space.memory(), space.base, pointer, false, false);
    observe(&space.memory(), space.base, pointer, false, false);
    {
        let mut store = STORE.get().unwrap().lock().unwrap();
        assert_eq!(store.batch.events.len(), 2);
        assert!(store.batch.failure.is_none());
        let later = store.batch.events[1];
        replace_copied_bytes(&mut store.batch, &later, pointer, &0_u64.to_le_bytes());
    }
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Err("variant vtable or reference type mismatch")
    );
    assert!(RECOVER.load(Ordering::Acquire));
    assert!(!baseline_admitted());
    assert_eq!(shared.len(), 1);
    let partial = Arc::clone(&shared.entries[0].value);
    assert_ne!(partial.value, previous[0].value);
    assert_eq!(retained(epoch, space.base).state_count(), 0);

    put(
        &mut space,
        pointer + native::MAP_RANGE_STAT,
        &stat.to_le_bytes(),
    );
    assert_eq!(
        native::RegistryRow::read(&space.memory(), space.base, pointer)
            .unwrap()
            .id,
        registry[0].id
    );
    prepare_fixture(&space, &registry);
    assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
    assert_eq!(retained(epoch, space.base).state_count(), 0);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert!(output.is_empty());
    assert_eq!(shared.len(), 0);
    assert_eq!(retained(epoch, space.base).state_count(), 0);
    source_shape(&mut space, pointer, 3, 3);
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert_eq!(shared.len(), 1);
    assert_eq!(shared.entries[0].value.value, previous[3].value);
    assert!(!Arc::ptr_eq(&shared.entries[0].value, &previous[3]));
    let grown = Arc::clone(&shared.entries[0].value);
    assert_eq!(retained(epoch, space.base).state_count(), 1);
    source_shape(&mut space, pointer, 3, 0);
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert_eq!(shared.len(), 1);
    assert_eq!(shared.entries[0].value.value, previous[0].value);
    assert!(!Arc::ptr_eq(&shared.entries[0].value, &previous[0]));
    assert!(!Arc::ptr_eq(&shared.entries[0].value, &partial));
    assert_eq!(retained(epoch, space.base).state_count(), 2);
    source_shape(&mut space, pointer, 0, 0);
    repair::mark(pointer);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(1)
    );
    assert_eq!(shared.len(), 0);
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].snapshot.as_ref().unwrap(), &previous[0].value);
    assert_eq!(retained(epoch, space.base).state_count(), 0);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert!(output.is_empty());
    assert_eq!(shared.len(), 0);
    assert_eq!(retained(epoch, space.base).state_count(), 0);
    source_shape(&mut space, pointer, 3, 3);
    observe(&space.memory(), space.base, pointer, false, false);
    assert_eq!(
        drain_shared(&space.memory(), space.base, 7, &mut output, &mut shared),
        Ok(0)
    );
    assert_eq!(shared.len(), 1);
    assert_eq!(shared.entries[0].value.value, grown.value);
    assert!(!Arc::ptr_eq(&shared.entries[0].value, &grown));
    assert_eq!(retained(epoch, space.base).state_count(), 1);
    drop(hooks);
    assert_eq!(retained(epoch, space.base).state_count(), 0);
}
