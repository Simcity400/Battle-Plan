use super::*;
use compact_contract::hourly_service::HourlyServiceOutcomeV1;

fn request(hour_raw: i32, observation_ordinal: u64) -> CaptureRequest {
    CaptureRequest {
        stamp: crate::read_only_scheduler::ClockStamp {
            epoch: 4,
            hour_raw,
            observation_ordinal,
        },
        force_baseline: false,
    }
}

fn staged(
    request: CaptureRequest,
    owner: ServiceOwner,
    degraded: bool,
    bytes: Vec<u8>,
) -> StagedPublication {
    StagedPublication {
        bytes,
        staged_after_frame: 2,
        kind: StagedPublicationKind::Data {
            request,
            degraded,
            service_owner: Some(owner),
        },
    }
}

fn complete_build_stages() {
    hourly_service_capture::set_stage(ServiceStage::Finish);
    hourly_service_capture::set_stage(ServiceStage::BuildTail);
    hourly_service_capture::set_stage(ServiceStage::PublicationPrepare);
}

#[test]
fn failed_staged_sink_closes_original_request_and_restores_current_owner() {
    let _session = hourly_service_capture::start();
    let previous = request(100, 8);
    let current = request(101, 9);
    let previous_owner = hourly_service_owner(previous, Some(73)).unwrap();
    let current_owner = hourly_service_owner(current, Some(73)).unwrap();
    begin_hourly_service(previous, Some(73));
    complete_build_stages();
    hourly_service_capture::suspend();
    begin_hourly_service(current, Some(73));
    let mut frames = vec![staged(previous, previous_owner, false, vec![1, 2, 3])];
    let mut transport = CaptureTransport {
        sinks: None,
        ordinary_control: None,
        injected_boundaries: std::collections::VecDeque::new(),
        injected_logical_stream_id: None,
        logical_stream_tail_record_type: None,
    };
    let mut results = Vec::new();
    run_staged_publications(&mut transport, &frames, &mut results, &mut 0, &[]);
    assert_eq!(results.len(), 1);
    let observed_sink_ns = results[0]
        .sink_finished
        .duration_since(results[0].sink_started)
        .as_nanos() as u64;
    let mut sequence = PublishedSequence::start_on(None);
    let mut closes = PendingStagedServiceCloses::default();
    assert!(
        resolve_staged_publications(&frames, results, &mut sequence, None, None, &mut closes)
            .is_err()
    );
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 0);
    assert!(!frames[0].bytes.is_empty());
    dispose_staged_publications(&mut frames, closes, &mut FrameBuffers::default());
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 1);
    assert_eq!(sequence.get(), 1);
    assert_eq!(hourly_service_capture::current_owner(), Some(current_owner));
    let report = hourly_service_capture::finish_with_scheduler_and_close(
        2,
        0,
        0,
        0,
        Some((current_owner, ServiceOutcome::Aborted, None)),
    )
    .unwrap();
    let previous = report
        .samples
        .iter()
        .find(|sample| sample.owner.game_hour_raw == 100)
        .unwrap();
    assert_eq!(previous.owner.logical_stream_id, 73);
    assert_eq!(previous.owner.epoch, 4);
    assert_eq!(previous.owner.observation_ordinal, 8);
    assert_eq!(previous.outcome, HourlyServiceOutcomeV1::Aborted);
    assert_eq!(previous.data_sequence, None);
    assert!(previous.stage_ns[ServiceStage::Sink as usize].unwrap() >= observed_sink_ns);
    assert!(previous.stage_ns[ServiceStage::Commit as usize].is_some());
    assert_eq!(report.census.closed_requests, 2);
}

#[test]
fn partial_staged_data_keeps_sink_interval_and_committed_sequence_bounds() {
    let path = std::env::temp_dir().join(format!(
        "hourly_service_adapter_{}_{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut transport =
        CaptureTransport::legacy(Some(file), None, SinkDeliveryState::new(true, false));
    let _session = hourly_service_capture::start();
    let request = request(100, 8);
    let owner = hourly_service_owner(request, Some(73)).unwrap();
    begin_hourly_service(request, Some(73));
    complete_build_stages();
    let mut bytes = Vec::new();
    append_compact_frame_end_record(&mut bytes, 1, 100, 0, 0, 0, 0, 0);
    let mut frames = vec![staged(request, owner, true, bytes.clone())];
    let mut results = Vec::new();
    run_staged_publications(&mut transport, &frames, &mut results, &mut 0, &[]);
    let observed_sink_ns = results[0]
        .sink_finished
        .duration_since(results[0].sink_started)
        .as_nanos() as u64;
    let mut sequence = PublishedSequence::start_on(None);
    let mut closes = PendingStagedServiceCloses::default();
    resolve_staged_publications(&frames, results, &mut sequence, None, None, &mut closes).unwrap();
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 0);
    assert_eq!(frames[0].bytes, bytes);
    dispose_staged_publications(&mut frames, closes, &mut FrameBuffers::default());
    assert!(frames.is_empty());
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 1);
    let report = hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).unwrap();
    transport.finish().unwrap();
    let actual = std::fs::read(&path).unwrap();
    portable_test_fs_retry::remove_file(&path).unwrap();
    assert_eq!(actual, bytes);
    assert_eq!(sequence.get(), 2);
    assert_eq!(report.samples.len(), 1);
    let sample = &report.samples[0];
    assert_eq!(sample.outcome, HourlyServiceOutcomeV1::Partial);
    assert_eq!(sample.data_sequence, Some((1, 2)));
    assert_eq!(sample.fault_bits, 0);
    assert!(sample.stage_ns[ServiceStage::Sink as usize].unwrap() >= observed_sink_ns);
    assert!(sample.stage_ns[ServiceStage::Commit as usize].is_some());
    assert_eq!(
        sample.not_applicable_stages,
        1 << ServiceStage::Terminal as u8
    );
    assert_eq!(report.census.partial_requests, 1);
    assert_eq!(report.census.unmeasured_hours, 0);
}

#[test]
fn missing_publisher_result_retains_an_incomplete_aborted_service_sample() {
    let _session = hourly_service_capture::start();
    let request = request(100, 8);
    let owner = hourly_service_owner(request, Some(73)).unwrap();
    begin_hourly_service(request, Some(73));
    complete_build_stages();
    let mut frames = vec![staged(request, owner, false, Vec::new())];
    let mut sequence = PublishedSequence::start_on(None);
    let mut closes = PendingStagedServiceCloses::default();
    assert!(resolve_staged_publications(
        &frames,
        Vec::new(),
        &mut sequence,
        None,
        None,
        &mut closes
    )
    .is_err());
    assert!(closes.entries.iter().all(Option::is_none));
    dispose_staged_publications(&mut frames, closes, &mut FrameBuffers::default());
    let report = hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).unwrap();
    assert_eq!(report.samples.len(), 1);
    assert_eq!(report.samples[0].owner.game_hour_raw, 100);
    assert_eq!(report.samples[0].outcome, HourlyServiceOutcomeV1::Aborted);
    assert_eq!(report.samples[0].data_sequence, None);
    assert_ne!(
        report.samples[0].fault_bits
            & (1 << hourly_service_capture::TimingFault::UnclosedRequest as u8),
        0
    );
    assert_eq!(report.census.admitted_requests, 1);
    assert_eq!(report.census.closed_requests, 1);
    assert_eq!(report.census.active_requests, 0);
    assert_ne!(
        report.census.fault_bits
            & (1 << hourly_service_capture::TimingFault::UnclosedRequest as u8),
        0
    );
    assert_eq!(sequence.get(), 1);
}

#[test]
fn missing_logical_stream_does_not_admit_a_synthetic_service_owner() {
    let _session = hourly_service_capture::start();
    begin_hourly_service(request(100, 8), None);
    assert_eq!(hourly_service_capture::current_owner(), None);
    let report = hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).unwrap();
    assert_eq!(report.census.admitted_requests, 0);
    assert_eq!(report.census.unmeasured_hours, 1);
    assert_ne!(
        report.census.fault_bits & (1 << hourly_service_capture::TimingFault::UnknownOwner as u8),
        0
    );
    assert!(report.samples.is_empty());
}

struct DisposalProbe<'a> {
    owner: ServiceOwner,
    dropped: &'a std::sync::atomic::AtomicBool,
}
impl Drop for DisposalProbe<'_> {
    fn drop(&mut self) {
        assert_eq!(hourly_service_capture::current_owner(), Some(self.owner));
        assert_eq!(hourly_service_capture::closed_requests_for_tests(), 0);
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn refused_pending_disposal_preserves_error_drop_order_and_successor() {
    let _session = hourly_service_capture::start();
    let previous = hourly_service_owner(request(100, 8), Some(73)).unwrap();
    let current = hourly_service_owner(request(101, 9), Some(73)).unwrap();
    hourly_service_capture::begin(previous, None);
    complete_build_stages();
    hourly_service_capture::suspend();
    hourly_service_capture::begin(current, None);
    let dropped = std::sync::atomic::AtomicBool::new(false);
    let result: Result<(), &str> = (|| {
        let mut disposal = Some(HourlyServiceDisposal::new(
            previous,
            ServiceStage::PublicationPrepare,
            false,
        ));
        let _buffer = DisposalProbe {
            owner: previous,
            dropped: &dropped,
        };
        queue_hourly_service_disposal(&mut disposal, Some(previous), ServiceOutcome::Refused, None);
        Err("original refusal")
    })();
    assert_eq!(result, Err("original refusal"));
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(hourly_service_capture::current_owner(), Some(current));
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 1);
    let report = hourly_service_capture::finish_with_scheduler_and_close(
        2,
        0,
        0,
        0,
        Some((current, ServiceOutcome::Aborted, None)),
    )
    .unwrap();
    assert_eq!(report.samples[0].outcome, HourlyServiceOutcomeV1::Refused);
    assert_eq!(report.samples[0].owner.game_hour_raw, 100);
}

#[test]
fn current_disposal_closes_after_buffers_and_returns_timing_to_setup() {
    let _session = hourly_service_capture::start();
    let owner = hourly_service_owner(request(100, 8), Some(73)).unwrap();
    hourly_service_capture::begin(owner, None);
    complete_build_stages();
    let dropped = std::sync::atomic::AtomicBool::new(false);
    {
        let mut disposal = Some(HourlyServiceDisposal::new(
            owner,
            ServiceStage::Commit,
            true,
        ));
        let _buffer = DisposalProbe {
            owner,
            dropped: &dropped,
        };
        queue_hourly_service_disposal(&mut disposal, Some(owner), ServiceOutcome::Aborted, None);
    }
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(hourly_service_capture::current_owner(), None);
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 1);
    let report = hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).unwrap();
    assert_eq!(report.samples[0].outcome, HourlyServiceOutcomeV1::Aborted);
}

#[test]
fn boundary_error_before_outcome_never_invents_a_disposal_completion() {
    let _session = hourly_service_capture::start();
    let owner = hourly_service_owner(request(100, 8), Some(73)).unwrap();
    hourly_service_capture::begin(owner, None);
    complete_build_stages();
    hourly_service_capture::suspend();
    let dropped = std::sync::atomic::AtomicBool::new(false);
    let result: Result<(), &str> = (|| {
        let _boundary = hourly_service_capture::unowned();
        let _disposal = HourlyServiceDisposal::new(owner, ServiceStage::Terminal, false);
        hourly_service_capture::claim_setup(owner, ServiceStage::Terminal);
        let _buffer = DisposalProbe {
            owner,
            dropped: &dropped,
        };
        Err("original boundary failure")
    })();
    assert_eq!(result, Err("original boundary failure"));
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 0);
    let report = hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).unwrap();
    assert_eq!(report.samples[0].outcome, HourlyServiceOutcomeV1::Aborted);
    assert_ne!(
        report.samples[0].fault_bits
            & (1 << hourly_service_capture::TimingFault::UnclosedRequest as u8),
        0
    );
}

#[test]
fn committed_data_survives_later_gap_failure_without_falsely_missing_a_stage() {
    let path = std::env::temp_dir().join(format!(
        "hourly_service_mixed_{}_{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut transport =
        CaptureTransport::legacy(Some(file), None, SinkDeliveryState::new(true, false));
    let _session = hourly_service_capture::start();
    let request = request(100, 8);
    let owner = hourly_service_owner(request, Some(73)).unwrap();
    begin_hourly_service(request, Some(73));
    complete_build_stages();
    let mut data = Vec::new();
    append_compact_frame_end_record(&mut data, 1, 100, 0, 0, 0, 0, 0);
    let mut gap = Vec::new();
    append_compact_frame_end_record(&mut gap, 2, 101, 0, 0, 0, 0, 0);
    let mut expected = data.clone();
    expected.extend_from_slice(&gap);
    let mut frames = vec![
        staged(request, owner, false, data),
        StagedPublication {
            bytes: gap,
            staged_after_frame: 3,
            kind: StagedPublicationKind::ControlGap { group_len: 1 },
        },
    ];
    let mut results = Vec::new();
    run_staged_publications(&mut transport, &frames, &mut results, &mut 0, &[]);
    let mut sequence = PublishedSequence::start_on(None);
    let mut closes = PendingStagedServiceCloses::default();
    let error =
        resolve_staged_publications(&frames, results, &mut sequence, None, None, &mut closes)
            .unwrap_err();
    assert_eq!(
        error,
        "a staged control-gap frame has no live reservation to commit"
    );
    assert_eq!(sequence.get(), 3);
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 0);
    dispose_staged_publications(&mut frames, closes, &mut FrameBuffers::default());
    let report = hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).unwrap();
    transport.finish().unwrap();
    let actual = std::fs::read(&path).unwrap();
    portable_test_fs_retry::remove_file(&path).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(report.samples[0].outcome, HourlyServiceOutcomeV1::Complete);
    assert_eq!(report.samples[0].data_sequence, Some((1, 2)));
    assert_eq!(
        report.samples[0].fault_bits
            & (1 << hourly_service_capture::TimingFault::MissingStages as u8),
        0,
    );
    assert!(report.samples[0].stage_ns[ServiceStage::Commit as usize].is_some());
    assert_eq!(report.census.unattributed_service_ns, 0);
    assert_eq!(
        report.census.fault_bits
            & (1 << hourly_service_capture::TimingFault::UnmeasuredHours as u8),
        0
    );
    assert_eq!(report.census.complete_measurements, 1);
    assert_eq!(report.census.incomplete_measurements, 0);
}

#[test]
fn stale_current_disposal_cannot_suspend_a_successor_session() {
    let first = hourly_service_capture::start();
    let previous = hourly_service_owner(request(100, 8), Some(73)).unwrap();
    hourly_service_capture::begin(previous, None);
    let stale = HourlyServiceDisposal::new(previous, ServiceStage::Commit, true);
    drop(first);
    let second = hourly_service_capture::start();
    let current = hourly_service_owner(request(101, 9), Some(74)).unwrap();
    hourly_service_capture::begin(current, None);
    drop(stale);
    assert_eq!(hourly_service_capture::current_owner(), Some(current));
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 0);
    let report = hourly_service_capture::finish_with_scheduler_and_close(
        1,
        0,
        0,
        0,
        Some((current, ServiceOutcome::Aborted, None)),
    )
    .unwrap();
    assert_eq!(report.samples.len(), 1);
    assert_eq!(report.samples[0].owner.logical_stream_id, 74);
    drop(second);
}

#[test]
fn multiple_staged_data_owners_each_close_their_complete_commit_stage() {
    let path = std::env::temp_dir().join(format!(
        "hourly_service_two_{}_{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let file = std::fs::File::create(&path).unwrap();
    let mut transport =
        CaptureTransport::legacy(Some(file), None, SinkDeliveryState::new(true, false));
    let _session = hourly_service_capture::start();
    let first = request(100, 8);
    let second = request(101, 9);
    let current = request(102, 10);
    let first_owner = hourly_service_owner(first, Some(73)).unwrap();
    let second_owner = hourly_service_owner(second, Some(73)).unwrap();
    let current_owner = hourly_service_owner(current, Some(73)).unwrap();
    for request in [first, second] {
        begin_hourly_service(request, Some(73));
        complete_build_stages();
        hourly_service_capture::suspend();
    }
    begin_hourly_service(current, Some(73));
    let mut data = Vec::new();
    append_compact_frame_end_record(&mut data, 1, 100, 0, 0, 0, 0, 0);
    let mut next = Vec::new();
    append_compact_frame_end_record(&mut next, 2, 101, 0, 0, 0, 0, 0);
    let mut expected = data.clone();
    expected.extend_from_slice(&next);
    let mut second_frame = staged(second, second_owner, true, next);
    second_frame.staged_after_frame = 3;
    let mut frames = vec![staged(first, first_owner, false, data), second_frame];
    let capacity = frames.capacity();
    let mut results = Vec::new();
    run_staged_publications(&mut transport, &frames, &mut results, &mut 0, &[]);
    let mut sequence = PublishedSequence::start_on(None);
    let mut closes = PendingStagedServiceCloses::default();
    resolve_staged_publications(&frames, results, &mut sequence, None, None, &mut closes).unwrap();
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 0);
    dispose_staged_publications(&mut frames, closes, &mut FrameBuffers::default());
    assert_eq!(hourly_service_capture::current_owner(), Some(current_owner));
    assert_eq!(hourly_service_capture::closed_requests_for_tests(), 2);
    assert!(frames.is_empty());
    assert_eq!(frames.capacity(), capacity);
    assert_eq!(sequence.get(), 3);
    let report = hourly_service_capture::finish_with_scheduler_and_close(
        3,
        0,
        0,
        0,
        Some((current_owner, ServiceOutcome::Aborted, None)),
    )
    .unwrap();
    for (index, outcome) in [
        HourlyServiceOutcomeV1::Complete,
        HourlyServiceOutcomeV1::Partial,
    ]
    .into_iter()
    .enumerate()
    {
        let sample = &report.samples[index];
        assert_eq!(sample.owner.game_hour_raw, 100 + index as i32);
        assert_eq!(sample.outcome, outcome);
        assert_eq!(
            sample.data_sequence,
            Some((index as u64 + 1, index as u64 + 2))
        );
        assert_eq!(
            sample.fault_bits & (1 << hourly_service_capture::TimingFault::MissingStages as u8),
            0,
        );
        assert!(sample.stage_ns[ServiceStage::Sink as usize].is_some());
        assert!(sample.stage_ns[ServiceStage::Commit as usize].is_some());
    }
    transport.finish().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    portable_test_fs_retry::remove_file(&path).unwrap();
}

#[test]
fn staged_close_without_matching_data_keeps_missing_disposal_fault() {
    let _session = hourly_service_capture::start();
    let request = request(100, 8);
    let owner = hourly_service_owner(request, Some(73)).unwrap();
    begin_hourly_service(request, Some(73));
    complete_build_stages();
    hourly_service_capture::set_stage(ServiceStage::Sink);
    let mut frames = vec![StagedPublication {
        bytes: vec![1, 2, 3],
        staged_after_frame: 1,
        kind: StagedPublicationKind::ControlGap { group_len: 1 },
    }];
    let mut closes = PendingStagedServiceCloses::default();
    closes.push(Some(owner), ServiceOutcome::Complete, Some((1, 2)));
    dispose_staged_publications(&mut frames, closes, &mut FrameBuffers::default());
    let report = hourly_service_capture::finish_with_scheduler(1, 0, 0, 0).unwrap();
    assert_eq!(report.samples.len(), 1);
    assert_ne!(
        report.samples[0].fault_bits
            & (1 << hourly_service_capture::TimingFault::MissingStages as u8),
        0
    );
    assert_eq!(report.census.complete_measurements, 0);
    assert_eq!(report.census.incomplete_measurements, 1);
}
