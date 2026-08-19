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

#[test]
fn capture_loop_result_and_panic_are_both_retained_as_abnormal() {
    let mut result_error = None;
    assert!(retain_capture_loop_outcome(
        Ok(Err("H4FR encoder failed".to_string())),
        &mut result_error,
    ));
    assert_eq!(result_error.as_deref(), Some("H4FR encoder failed"));

    let panic_outcome =
        std::panic::catch_unwind(|| -> Result<(), String> { panic!("capture fixture panic") });
    let mut panic_error = None;
    assert!(retain_capture_loop_outcome(panic_outcome, &mut panic_error,));
    assert_eq!(
        panic_error.as_deref(),
        Some("capture loop panicked: capture fixture panic")
    );

    let mut clean_error = None;
    assert!(!retain_capture_loop_outcome(Ok(Ok(())), &mut clean_error));
    assert!(clean_error.is_none());
}

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

    let freeze = vec![0u8; 8];
    let census = compact_contract::content_identity_census::ContentIdentityCensusV1 {
        buildings: vec![
            compact_contract::content_identity_census::ContentIdentityRow {
                token_id: 0x4E22,
                token: "arms_factory".to_string(),
            },
        ],
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
            (
                3,
                compact_contract::COMPACT_RECORD_TYPE_CAPTURE_TERMINATED,
                0
            ),
        ],
        "the successor stream carries both preamble records, in order, before the notice"
    );
}
