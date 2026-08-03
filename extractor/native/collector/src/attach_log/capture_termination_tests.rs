use super::*;

fn test_scheduler_runtime(
    unexpected_exit: bool,
    handle: thread::JoinHandle<()>,
) -> ReadOnlySchedulerRuntime {
    let scheduler = ReadOnlyScheduler::new(vec![18], 1, 8).expect("test scheduler");
    ReadOnlySchedulerRuntime {
        shared: Arc::new((Mutex::new(scheduler), Condvar::new())),
        shutdown: Arc::new(AtomicBool::new(false)),
        unexpected_exit: Arc::new(AtomicBool::new(unexpected_exit)),
        handle: Some(handle),
        date_addr: 0,
        module_base: 0,
    }
}

/// Mutation proof: changing the production H4AT envelope to reuse the current
/// game hour, H4DG/H4FR type, or a scheduled domain makes this fail. Those
/// mutations would turn a process abort into a false coverage statement.
#[test]
fn termination_envelope_is_frame_independent_and_claims_no_hour() {
    let reasons = compact_contract::capture_termination::CaptureTerminationReasons::from_bits(
        compact_contract::capture_termination::CAPTURE_TERMINATION_REASON_CAPTURE_LOOP_ABORTED,
    )
    .unwrap();
    let bytes = encode_capture_termination_h4cr(17, reasons, 1900, "H4FR encoder failed");
    let header = compact_contract::parse_compact_record_header(&bytes).unwrap();
    assert_eq!(header.sequence, 17);
    assert_eq!(header.game_hour_raw, 0);
    assert_eq!(header.domain_id, compact_contract::COMPACT_DOMAIN_FRAME);
    assert_eq!(
        header.record_type,
        compact_contract::COMPACT_RECORD_TYPE_CAPTURE_TERMINATED
    );
    let decoded = compact_contract::capture_termination::CaptureTerminationRecordV1::decode(
        &bytes[compact_contract::COMPACT_RECORD_HEADER_LEN..],
    )
    .unwrap();
    assert_eq!(decoded.reasons, reasons);
    assert_eq!(decoded.detail, "H4FR encoder failed");
}

/// Mutation proof: restoring `let _ = handle.join()` makes both typed outcomes
/// disappear and fails these assertions. Ignoring the early-exit witness makes
/// the first assertion fail; ignoring JoinHandle::Err makes the second fail.
#[test]
fn observer_join_surfaces_early_exit_and_panic() {
    let early = thread::spawn(|| {});
    let mut early_runtime = test_scheduler_runtime(true, early);
    assert_eq!(
        early_runtime.stop_observer_thread(),
        Some(ObserverThreadTermination::ExitedEarly)
    );

    let panicked = thread::spawn(|| panic!("observer fixture panic"));
    let mut panic_runtime = test_scheduler_runtime(false, panicked);
    assert_eq!(
        panic_runtime.stop_observer_thread(),
        Some(ObserverThreadTermination::Panicked(
            "observer fixture panic".to_string()
        ))
    );
}

/// Mutation proof: removing the production `catch_unwind` boundary no longer
/// type-checks at its call to this helper; treating either propagated errors or
/// panics as normal makes the corresponding assertion fail and suppresses H4AT.
#[test]
fn capture_loop_result_and_panic_are_both_retained_as_abnormal() {
    let mut result_error = None;
    assert!(retain_capture_loop_outcome(
        Ok(Err("H4FR encoder failed".to_string())),
        &mut result_error,
    ));
    assert_eq!(result_error.as_deref(), Some("H4FR encoder failed"));

    let panic_outcome = std::panic::catch_unwind(|| -> Result<(), String> {
        panic!("capture fixture panic")
    });
    let mut panic_error = None;
    assert!(retain_capture_loop_outcome(
        panic_outcome,
        &mut panic_error,
    ));
    assert_eq!(
        panic_error.as_deref(),
        Some("capture loop panicked: capture fixture panic")
    );

    let mut clean_error = None;
    assert!(!retain_capture_loop_outcome(Ok(Ok(())), &mut clean_error));
    assert!(clean_error.is_none());
}
