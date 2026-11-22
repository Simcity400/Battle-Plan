use super::*;

struct Menu;
impl Memory for Menu {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address != engine_native::cgame_state::SINGLETON_RVA.0 || output.len() != 8 {
            return false;
        }
        output.fill(0);
        true
    }
}

fn assert_burst(output: &[Entry], count: usize) {
    assert!(
        output[..count]
            .iter()
            .enumerate()
            .all(|(index, entry)| entry.line == index as u64 + 1 && entry.amount == index as i64)
    );
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
fn zero_native_demand_installs_empty_storage_and_a_burst_into_it_is_kept_whole() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    // SAFETY: Test holds `test_lock()` with construction-IC hooks quiescent.
    unsafe { reset_log_for_test(0) };
    *LOG_SWAP.lock().unwrap() = LogSwap::default();
    LOG.store(ptr::null_mut(), Ordering::SeqCst);
    assert_eq!(presize_from_live(&Menu, 0), Ok(()));
    unsafe { install_offered() };
    assert!(log().is_some());
    assert_eq!(capacity(), 0);
    let mut output = Vec::new();
    assert!(drain(&mut output).unwrap().complete());

    let spilled = SPILLED.load(Ordering::Relaxed);
    append_burst(10_003);
    assert_eq!(SPILLED.load(Ordering::Relaxed) - spilled, 10_003);
    assert_eq!(OBSERVATION.required.load(Ordering::Acquire), 10_003);
    let kept = drain(&mut output).unwrap();
    assert_eq!((kept.pending, kept.copied), (10_003, 10_003));
    assert!(kept.complete());
    assert_burst(&output, 10_003);
    presize(0);
    unsafe { install_offered() };
    assert_eq!(capacity(), 10_003);
    collect();
    let grown = drain(&mut output).unwrap();
    assert_eq!((grown.epoch, grown.first), (kept.epoch, kept.cursor));
    let spilled = SPILLED.load(Ordering::Relaxed);
    append_burst(10_003);
    assert_eq!(SPILLED.load(Ordering::Relaxed), spilled);
    let ringed = drain(&mut output).unwrap();
    assert!(ringed.complete());
    assert_eq!(ringed.copied, 10_003);
    assert_burst(&output, 10_003);
    observe_for_test(false, 0);
}

#[test]
fn live_growth_preserves_pending_entries_cursor_epoch_and_retired_storage() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    // SAFETY: Test holds `test_lock()` with construction-IC hooks quiescent.
    unsafe { reset_log_for_test(5_000) };
    append_burst(5_000);
    let cursor = CURSOR.load(Ordering::SeqCst);
    let epoch = OBSERVATION.epoch.load(Ordering::SeqCst);
    presize(10_003);
    unsafe { install_offered() };
    assert_eq!(capacity(), 5_000);
    let mut output = Vec::new();
    assert_eq!(drain(&mut output).unwrap().copied, 5_000);
    let writer = OBSERVATION.enter();
    unsafe { install_offered() };
    assert_eq!(capacity(), 5_000);
    drop(writer);
    unsafe { install_offered() };
    assert_eq!(capacity(), 20_006);
    assert_eq!(CURSOR.load(Ordering::SeqCst), cursor);
    assert_eq!(CONSUMED.load(Ordering::Relaxed), cursor);
    assert_eq!(OBSERVATION.epoch.load(Ordering::SeqCst), epoch);
    assert!(LOG_SWAP.lock().unwrap().retired.is_some());
    presize(30_003);
    unsafe { install_offered() };
    assert_eq!(capacity(), 20_006);
    collect();
    unsafe { install_offered() };
    assert_eq!(capacity(), 60_006);
    collect();
    append_burst(30_003);
    let result = drain(&mut output).unwrap();
    assert_eq!(
        (result.epoch, result.first, result.copied),
        (epoch, cursor, 30_003)
    );
    assert!(result.complete());
    observe_for_test(false, 0);
}

#[test]
fn a_burst_past_the_log_is_kept_whole_and_its_demand_survives_consumption() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    // SAFETY: Test holds `test_lock()` with construction-IC hooks quiescent.
    unsafe { reset_log_for_test(5_000) };
    append_burst(70_003);
    let mut output = Vec::new();
    let loss = drain(&mut output).unwrap();
    assert_eq!((loss.copied, loss.pending), (70_003, 70_003));
    assert_burst(&output, 70_003);
    assert_eq!(OBSERVATION.required.load(Ordering::Acquire), 70_003);
    presize(1);
    unsafe { install_offered() };
    collect();
    assert_eq!(capacity(), 70_003);
    let after = drain(&mut output).unwrap();
    assert_eq!((after.epoch, after.first), (loss.epoch, loss.cursor));
    observe_for_test(false, 0);
}

#[test]
fn failed_live_hint_retains_full_original_burst_pressure() {
    let _lock = test_lock();
    observe_for_test(true, 0);
    // SAFETY: Test holds `test_lock()` with construction-IC hooks quiescent.
    unsafe { reset_log_for_test(0) };
    append_burst(10_003);
    let mut output = Vec::new();
    let loss = drain(&mut output).unwrap();
    assert_eq!(loss.copied, 10_003);
    assert_burst(&output, 10_003);
    let missing = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    assert_eq!(
        presize_from_live(&missing, 0),
        Err("construction output sizing game state unreadable"),
    );
    assert!(!REFUSAL_PENDING.load(Ordering::SeqCst));
    unsafe { install_offered() };
    collect();
    assert_eq!(capacity(), 10_003);
    let after = drain(&mut output).unwrap();
    assert_eq!((after.epoch, after.first), (loss.epoch, loss.cursor));
    observe_for_test(false, 0);
}
