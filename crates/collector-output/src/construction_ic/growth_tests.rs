use super::*;

struct Menu;
impl Memory for Menu {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address != engine_native::cgame_state::SINGLETON_RVA || output.len() != 8 {
            return false;
        }
        output.fill(0);
        true
    }
}

fn append_burst(count: usize) {
    let _writer = OBSERVATION.enter();
    for index in 0..count {
        append(Entry {
            line: index as u64 + 1,
            amount: index as i64,
            ..Entry::default()
        });
    }
}

#[test]
fn zero_native_demand_installs_empty_storage_and_full_rejected_bursts_grow() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    reset_log_for_test(0);
    *LOG_SWAP.lock().unwrap() = LogSwap::default();
    LOG.store(ptr::null_mut(), Ordering::SeqCst);
    assert_eq!(presize_from_live(&Menu, 0), Ok(()));
    install_offered();
    assert!(log().is_some());
    assert_eq!(capacity(), 0);
    let mut output = Vec::new();
    assert!(drain_growing(&mut output).unwrap().complete());

    let rejected = REJECTED.load(Ordering::Relaxed);
    append_burst(10_003);
    assert_eq!(REJECTED.load(Ordering::Relaxed) - rejected, 10_003);
    assert_eq!(OBSERVATION.required.load(Ordering::Acquire), 10_003);
    let loss = drain_growing(&mut output).unwrap();
    assert_eq!(
        (loss.pending, loss.copied, loss.first),
        (10_003, 0, loss.cursor)
    );
    presize(0);
    install_offered();
    assert_eq!(capacity(), 10_003);
    collect();
    let grown = drain_growing(&mut output).unwrap();
    assert_eq!((grown.epoch, grown.first), (loss.epoch, loss.cursor));
    append_burst(10_003);
    let recovered = drain_growing(&mut output).unwrap();
    assert!(recovered.complete());
    assert_eq!(recovered.copied, 10_003);
    assert_eq!(output[10_002].line, 10_003);
    observe_for_test(false, 0);
}

#[test]
fn first_worker_drain_keeps_five_thousand_and_mod_sized_event_populations() {
    let _lock = test_lock();
    for events in [5_000, 5_000 * 31] {
        observe_for_test(true, 0);
        reset_log_for_test(0);
        presize(events);
        install_offered();
        assert_eq!(capacity(), events * 2);
        collect();
        append_burst(events);
        let mut output = Vec::new();
        let result = drain_growing(&mut output).unwrap();
        assert!(result.complete());
        assert_eq!(
            (result.pending, result.copied, output.len()),
            (events, events, events)
        );
        for (index, row) in output.iter().enumerate() {
            assert_eq!((row.line, row.amount), (index as u64 + 1, index as i64));
        }
        assert_eq!(drain_growing(&mut output).unwrap().copied, 0);
        observe_for_test(false, 0);
    }
}

#[test]
fn live_growth_preserves_pending_entries_cursor_epoch_and_retired_storage() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    reset_log_for_test(5_000);
    append_burst(5_000);
    let cursor = CURSOR.load(Ordering::SeqCst);
    let epoch = OBSERVATION.epoch.load(Ordering::SeqCst);
    presize(10_003);
    install_offered();
    assert_eq!(capacity(), 5_000);
    let mut output = Vec::new();
    assert_eq!(drain_growing(&mut output).unwrap().copied, 5_000);
    let writer = OBSERVATION.enter();
    install_offered();
    assert_eq!(capacity(), 5_000);
    drop(writer);
    install_offered();
    assert_eq!(capacity(), 20_006);
    assert_eq!(CURSOR.load(Ordering::SeqCst), cursor);
    assert_eq!(CONSUMED.load(Ordering::Relaxed), cursor);
    assert_eq!(OBSERVATION.epoch.load(Ordering::SeqCst), epoch);
    assert!(LOG_SWAP.lock().unwrap().retired.is_some());
    presize(30_003);
    install_offered();
    assert_eq!(capacity(), 20_006);
    collect();
    install_offered();
    assert_eq!(capacity(), 60_006);
    collect();
    append_burst(30_003);
    let result = drain_growing(&mut output).unwrap();
    assert_eq!(
        (result.epoch, result.first, result.copied),
        (epoch, cursor, 30_003)
    );
    assert!(result.complete());
    observe_for_test(false, 0);
}

#[test]
fn original_overflow_demand_survives_consumption_with_unchanged_native_hint() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    reset_log_for_test(5_000);
    append_burst(70_003);
    let mut output = Vec::new();
    let loss = drain_growing(&mut output).unwrap();
    assert_eq!((loss.copied, loss.pending), (0, 70_003));
    assert!(output.is_empty());
    assert_eq!(OBSERVATION.required.load(Ordering::Acquire), 70_003);
    presize(1);
    install_offered();
    collect();
    assert_eq!(capacity(), 70_003);
    let after = drain_growing(&mut output).unwrap();
    assert_eq!((after.epoch, after.first), (loss.epoch, loss.cursor));
    observe_for_test(false, 0);
}

#[test]
fn failed_live_hint_is_counted_and_retains_full_original_burst_pressure() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    reset_log_for_test(0);
    append_burst(10_003);
    let mut output = Vec::new();
    let loss = drain_growing(&mut output).unwrap();
    let before = serde_json::to_value(demand::status()).unwrap()["failed_observations"]
        .as_u64()
        .unwrap();
    let missing = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    assert_eq!(
        presize_from_live(&missing, 0),
        Err("construction output sizing game state unreadable"),
    );
    let failed = serde_json::to_value(demand::status()).unwrap();
    assert_eq!(failed["failed_observations"], before + 1);
    assert_eq!(
        failed["last_reason"],
        "construction output sizing game state unreadable"
    );
    assert!(!REFUSAL_PENDING.load(Ordering::SeqCst));
    install_offered();
    collect();
    assert_eq!(capacity(), 10_003);
    let after = drain_growing(&mut output).unwrap();
    assert_eq!((after.epoch, after.first), (loss.epoch, loss.cursor));
    observe_for_test(false, 0);
}
