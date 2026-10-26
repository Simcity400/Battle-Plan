use super::*;

fn pending(
    pointer: usize,
    count: u32,
    capacity: u32,
) -> (super::super::Entry, HashMap<u64, SourceHeads>) {
    assert!(count <= capacity);
    let mut bytes = [0; native::STATS];
    bytes[native::BONUS_SOURCES..][..8].copy_from_slice(&0x100_0000_u64.to_le_bytes());
    bytes[native::BONUS_SOURCES + 8..][..4].copy_from_slice(&capacity.to_le_bytes());
    bytes[native::BONUS_SOURCES + 12..][..4].copy_from_slice(&count.to_le_bytes());
    let memory = engine_native::memory::SliceMemory {
        base: pointer,
        bytes: &bytes,
    };
    let mut batch = Batch::empty();
    batch.prepare(1, 1, bytes.len());
    batch
        .log
        .take_bulk(&memory, pointer, bytes.len(), 1)
        .unwrap();
    let observed = witness::Observed::new(&memory, 1);
    assert!(observed.copy(pointer + native::BONUS_SOURCES, &mut [0; 16]));
    let witness = observed.finish(&batch.log, 0..1, &[0], true).unwrap();
    let mut definition = native::DefinitionRow::default();
    definition.identity.id = 1;
    definition.design = Arc::new(rows::VariantDesignState {
        bonus_sources: vec![1; count as usize],
        ..Default::default()
    });
    let value = Arc::new(Decoded {
        value: Default::default(),
        definition,
        context: None,
    });
    let mut heads = HashMap::new();
    assert!(Cache::verified(&mut heads, pointer, 0..1, &value, Some(0)));
    (
        super::super::Entry {
            range: 0..1,
            pointer,
            next: None,
            value: Arc::clone(&value),
            witness: Some(witness),
            witness_next: None,
            missions: None,
            mission_next: None,
        },
        heads,
    )
}
fn remember(retained: &mut Retained, pointer: usize, count: u32, capacity: u32) -> Arc<Decoded> {
    let (entry, heads) = pending(pointer, count, capacity);
    let value = Arc::clone(&entry.value);
    retained.observe(1, pointer, Some(capacity));
    retained.remember(vec![entry], &heads);
    value
}

#[test]
fn unchanged_capacity_refreshes_fifteen_hundred_states_without_scanning_them() {
    let mut retained = Retained::default();
    for count in 0..1_500 {
        remember(&mut retained, 0x1000, count, 1_500);
    }
    assert_eq!(retained.state_count(), 1_500);
    assert_eq!(retained.capacity_visits, 0);
    let first = Arc::clone(&retained.designs[&1].states[&0].value);
    for count in 0..1_500 {
        let refreshed = remember(&mut retained, 0x1000, count, 1_500);
        assert!(Arc::ptr_eq(
            &retained.designs[&1].states[&count].value,
            &refreshed
        ));
        assert_eq!(retained.state_count(), 1_500);
        assert_eq!(retained.capacity_visits, 0);
    }
    assert!(!Arc::ptr_eq(&retained.designs[&1].states[&0].value, &first));
    remember(&mut retained, 0x1000, 749, 749);
    assert_eq!(retained.state_count(), 750);
    assert_eq!(retained.capacity_visits, 1_500);
    assert!(
        retained.designs[&1]
            .states
            .keys()
            .all(|count| *count <= 749)
    );
    remember(&mut retained, 0x1000, 0, 749);
    assert_eq!(retained.capacity_visits, 1_500);
    remember(&mut retained, 0x1000, 1_499, 1_500);
    assert_eq!(retained.designs[&1].capacity, 1_500);
    assert_eq!(retained.state_count(), 751);
    assert_eq!(retained.capacity_visits, 1_500);
    remember(&mut retained, 0x1000, 1_200, 1_200);
    assert_eq!(retained.state_count(), 751);
    assert_eq!(retained.capacity_visits, 2_251);
    assert!(!retained.designs[&1].states.contains_key(&1_499));
    remember(&mut retained, 0x2000, 0, 12);
    assert_eq!(retained.designs[&1].pointer, 0x2000);
    assert_eq!(retained.designs[&1].capacity, 12);
    assert_eq!(retained.state_count(), 1);
    assert_eq!(retained.capacity_visits, 2_251);
    remember(&mut retained, 0x2000, 12, 12);
    remember(&mut retained, 0x2000, 0, 0);
    assert_eq!(retained.state_count(), 1);
    assert_eq!(retained.capacity_visits, 2_253);
}

#[test]
fn capacity_records_cannot_be_recreated_or_raised_by_optional_promotion() {
    thread_local! { static TERMINAL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    fn terminal() {
        TERMINAL.with(|calls| calls.set(calls.get() + 1));
    }
    struct Hook;
    impl Drop for Hook {
        fn drop(&mut self) {
            engine_allocation::set_hooks(None, None);
        }
    }
    let mut retained = Retained::default();
    let (entry, heads) = pending(0x1000, 3, 3);
    let _hook = Hook;
    engine_allocation::set_hooks(None, Some(terminal));
    collector_test_support::with_allocations_denied(|| retained.observe(1, 0x1000, Some(3)));
    assert_eq!(collector_test_support::DENIALS.load(Ordering::Relaxed), 1);
    assert!(retained.designs.is_empty());
    retained.remember(vec![entry], &heads);
    assert!(retained.designs.is_empty());
    remember(&mut retained, 0x1000, 0, 0);
    for count in 1..=3 {
        remember(&mut retained, 0x1000, count, 3);
    }
    assert_eq!(retained.state_count(), 4);
    let (entry, heads) = pending(0x1000, 3, 3);
    let entries = vec![entry];
    collector_test_support::with_allocations_denied(|| {
        retained.observe(1, 0x1000, Some(0));
        retained.remember(entries, &heads);
    });
    assert_eq!(collector_test_support::DENIALS.load(Ordering::Relaxed), 0);
    assert_eq!(retained.designs[&1].capacity, 0);
    assert_eq!(retained.state_count(), 1);
    assert_eq!(retained.capacity_visits, 4);
    let (entry, heads) = pending(0x1000, 0, 0);
    let entries = vec![entry];
    collector_test_support::with_allocations_denied(|| {
        retained.observe(1, 0x1000, None);
        retained.remember(entries, &heads);
    });
    assert_eq!(collector_test_support::DENIALS.load(Ordering::Relaxed), 0);
    assert!(retained.designs.is_empty());
    remember(&mut retained, 0x1000, 3, 3);
    collector_test_support::with_allocations_denied(|| retained.observe(1, 0x2000, Some(0)));
    assert_eq!(collector_test_support::DENIALS.load(Ordering::Relaxed), 0);
    assert_eq!(retained.designs[&1].pointer, 0x2000);
    assert_eq!(retained.designs[&1].capacity, 0);
    assert_eq!(retained.state_count(), 0);
    let (entry, heads) = pending(0x1000, 3, 3);
    retained.remember(vec![entry], &heads);
    assert_eq!(retained.state_count(), 0);
    assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
}
