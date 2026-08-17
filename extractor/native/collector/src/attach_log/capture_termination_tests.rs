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

/// kills: deleting the content-identity census publication from the BODY of
/// `publish_capture_termination_record`. That path opens a successor logical
/// stream of its own, and a reader may open that member without ever seeing its
/// predecessor - so a preamble carrying only the install freeze census would
/// give that stream a session with no content identity. Counting caller
/// arguments cannot see this deletion; publishing through the production
/// function and reading the bytes back can.
#[test]
fn a_terminated_successor_stream_carries_both_preamble_records_before_the_notice() {
    let path = std::env::temp_dir().join(format!(
        "collector_termination_preamble_{}_{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut transport =
        CaptureTransport::legacy(Some(file), None, SinkDeliveryState::new(true, false));
    let mut sequence = PublishedSequence::start_on(Some(0));
    transport.inject_current_logical_stream_id(Some(1));

    // The freeze census's own payload grammar is pinned by its own tests; what
    // this case is about is WHICH records the successor preamble contains and
    // in what order, which the H4CR headers alone answer.
    let freeze = vec![0u8; 8];
    let census = compact_contract::content_identity_census::ContentIdentityCensusV1 {
        buildings: vec![compact_contract::content_identity_census::ContentIdentityRow {
            token_id: 0x4E22,
            token: "arms_factory".to_string(),
        }],
        resources: Vec::new(),
        buildings_declared: 1,
        resources_declared: 0,
        status: 0,
    }
    .encode()
    .unwrap();
    let reasons = compact_contract::capture_termination::CaptureTerminationReasons::from_bits(
        compact_contract::capture_termination::CAPTURE_TERMINATION_REASON_CAPTURE_LOOP_ABORTED,
    )
    .unwrap();

    publish_capture_termination_record(
        &mut transport,
        &mut sequence,
        Some(&freeze),
        Some(&census),
        reasons,
        1900,
        "capture loop aborted",
    )
    .unwrap();
    transport.finish().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    portable_test_fs_retry::remove_file(&path).unwrap();
    let mut at = 0;
    let mut seen = Vec::new();
    while at < bytes.len() {
        let header = compact_contract::parse_compact_record_header(&bytes[at..]).unwrap();
        seen.push((header.sequence, header.record_type, header.game_hour_raw));
        at += header.record_len();
    }
    assert_eq!(
        seen,
        vec![
            (1, compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS, 0),
            (
                2,
                compact_contract::COMPACT_RECORD_TYPE_CONTENT_IDENTITY_CENSUS,
                0
            ),
            (3, compact_contract::COMPACT_RECORD_TYPE_CAPTURE_TERMINATED, 0),
        ],
        "the successor stream carries both preamble records, in order, before the notice"
    );
}
