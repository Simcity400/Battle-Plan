use super::*;

#[test]
fn a_final_shrink_reusing_an_earlier_occurrence_limits_every_promoted_or_retained_state() {
    for warm in [false, true] {
        for full_copy in [false, true] {
            let mut batch = changing_source_shapes(5);
            for index in [0, 4] {
                let event = batch.events[index];
                replace_copied_bytes(
                    &mut batch,
                    &event,
                    event.pointer + native::BONUS_SOURCES + 8,
                    &0_u32.to_le_bytes(),
                );
            }
            if full_copy {
                batch.events[4].start = batch.events[0].start;
                batch.events[4].end = batch.events[0].end;
            }
            let mut retained = Retained::default();
            let mut shared = Shared::default();
            if warm {
                let mut cache = Cache::with_retained(&mut retained);
                for event in &batch.events[..4] {
                    cache.decode(&batch, event, 0, 7, &mut shared).unwrap();
                }
                cache.finish(&batch);
                drop(cache);
                assert_eq!(retained.state_count(), 4);
            }
            let mut cache = Cache::with_retained(&mut retained);
            let mut values = Vec::new();
            for event in &batch.events[..4] {
                values.push(cache.decode(&batch, event, 0, 751, &mut shared).unwrap());
            }
            let before = cache.counts;
            let last = cache
                .decode(&batch, &batch.events[4], 0, 751, &mut shared)
                .unwrap();
            assert!(Arc::ptr_eq(&last, &values[0]));
            assert_eq!(
                cache.counts[CacheOutcome::FullCopyHit as usize]
                    - before[CacheOutcome::FullCopyHit as usize],
                u64::from(full_copy)
            );
            assert_eq!(
                cache.counts[CacheOutcome::ReadWitnessHit as usize]
                    - before[CacheOutcome::ReadWitnessHit as usize],
                u64::from(!full_copy)
            );
            assert_eq!(cache.entries.len(), if warm { 3 } else { 4 });
            assert_eq!(
                cache.retained.as_ref().unwrap().state_count(),
                usize::from(warm)
            );
            cache.finish(&batch);
            drop(cache);
            assert_eq!(retained.state_count(), 1);
            let mut cache = Cache::with_retained(&mut retained);
            let zero = cache
                .decode(&batch, &batch.events[0], 0, 1_495, &mut shared)
                .unwrap();
            assert!(Arc::ptr_eq(&zero, &last));
            assert!(cache.entries.is_empty());
            assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 0);
            cache.finish(&batch);
            drop(cache);
            assert_eq!(retained.state_count(), 1);
            let mut cache = Cache::with_retained(&mut retained);
            let grown = cache
                .decode(&batch, &batch.events[3], 0, 2_239, &mut shared)
                .unwrap();
            assert_eq!(grown.value, values[3].value);
            assert!(!Arc::ptr_eq(&grown, &values[3]));
            assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 1);
            cache.finish(&batch);
            drop(cache);
            assert_eq!(retained.state_count(), 2);
        }
    }
}

#[test]
fn a_retained_shrink_survives_denial_of_either_candidate_map_and_quiet_drains() {
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
    let mut batch = changing_source_shapes(4);
    let zero = batch.events[0];
    replace_copied_bytes(
        &mut batch,
        &zero,
        zero.pointer + native::BONUS_SOURCES + 8,
        &0_u32.to_le_bytes(),
    );
    let _hook = Hook;
    engine_allocation::set_hooks(None, Some(terminal));
    for ordinal in [1, 2] {
        let mut retained = Retained::default();
        let mut shared = Shared::default();
        let mut values = Vec::new();
        for range in [0..1, 1..4] {
            let mut cache = Cache::with_retained(&mut retained);
            for event in &batch.events[range] {
                values.push(cache.decode(&batch, event, 0, 7, &mut shared).unwrap());
            }
            cache.finish(&batch);
        }
        assert_eq!(retained.state_count(), 4);
        Cache::observe(&values[0]).unwrap();
        let mut cache = Cache::with_retained(&mut retained);
        // Only the candidate-map reservations may allocate during this retained hit.
        engine_allocation::reserve(&mut cache.order, batch.log.spans.len()).unwrap();
        engine_allocation::reserve(&mut cache.comparison, batch.log.bytes.len()).unwrap();
        let (actual, denials) = {
            let _guard = collector_test_support::NullGuard::new(ordinal);
            let actual = cache.decode(&batch, &zero, 0, 751, &mut shared);
            (
                actual,
                collector_test_support::DENIALS.load(Ordering::Relaxed),
            )
        };
        let actual = actual.unwrap();
        assert!(Arc::ptr_eq(&actual, &values[0]));
        assert_eq!(denials, 1);
        assert!(cache.source_heads.is_empty());
        assert!(cache.entries.is_empty());
        assert_eq!(cache.counts[CacheOutcome::ReadWitnessHit as usize], 1);
        assert_eq!(cache.counts[CacheOutcome::FullDecode as usize], 0);
        assert_eq!(cache.counts[CacheOutcome::AllocationSkipped as usize], 1);
        assert_eq!(cache.retained.as_ref().unwrap().state_count(), 1);
        collector_test_support::with_allocations_denied(|| cache.finish(&batch));
        assert_eq!(collector_test_support::DENIALS.load(Ordering::Relaxed), 0);
        drop(cache);
        assert_eq!(retained.state_count(), 1);
        let mut quiet = Cache::with_retained(&mut retained);
        collector_test_support::with_allocations_denied(|| quiet.finish(&Batch::empty()));
        assert_eq!(collector_test_support::DENIALS.load(Ordering::Relaxed), 0);
        drop(quiet);
        assert_eq!(retained.state_count(), 1);
        let mut grown = Cache::with_retained(&mut retained);
        let actual = grown
            .decode(&batch, &batch.events[3], 0, 1_495, &mut shared)
            .unwrap();
        assert_eq!(actual.value, values[3].value);
        assert!(!Arc::ptr_eq(&actual, &values[3]));
        assert_eq!(grown.counts[CacheOutcome::FullDecode as usize], 1);
        grown.finish(&batch);
        drop(grown);
        assert_eq!(retained.state_count(), 2);
        assert_eq!(TERMINAL.with(std::cell::Cell::get), 0);
    }
}

#[test]
fn a_capacity_observation_without_a_complete_root_evicts_even_when_old_entries_can_promote() {
    let mut batch = changing_source_shapes(1);
    let event = batch.events[0];
    let mut retained = Retained::default();
    let mut shared = Shared::default();
    let mut warm = Cache::with_retained(&mut retained);
    warm.decode(&batch, &event, 0, 7, &mut shared).unwrap();
    warm.finish(&batch);
    drop(warm);
    replace_copied_bytes(
        &mut batch,
        &event,
        event.pointer + native::MAP_RANGE_STAT,
        &1234_i64.to_le_bytes(),
    );
    let mut cache = Cache::with_retained(&mut retained);
    cache.decode(&batch, &event, 0, 751, &mut shared).unwrap();
    assert_eq!(cache.entries.len(), 1);
    batch.log.spans[event.start].ok = false;
    assert!(cache.decode(&batch, &event, 0, 751, &mut shared).is_err());
    assert_eq!(cache.retained.as_ref().unwrap().state_count(), 0);
    cache.finish(&batch);
    drop(cache);
    assert_eq!(retained.state_count(), 0);
}
