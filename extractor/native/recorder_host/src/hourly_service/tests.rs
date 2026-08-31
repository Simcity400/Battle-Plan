use super::*;
use crate::control_transport::SessionId;
use compact_contract::hourly_service::HourlyServiceOwnerV1;
use std::sync::atomic::{AtomicU64, Ordering};

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hoi4-hourly-service-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn binding() -> HourlyServiceBinding {
    HourlyServiceBinding::new([0x51; 16], 41, 99, [0x31; 32])
}

fn sample(hour: i32, first: u64, ns: u64) -> HourlyServiceSampleV1 {
    HourlyServiceSampleV1 {
        owner: HourlyServiceOwnerV1 {
            logical_stream_id: 0,
            epoch: 0,
            game_hour_raw: hour,
            observation_ordinal: hour as u64,
        },
        outcome: HourlyServiceOutcomeV1::Complete,
        data_sequence: Some((first, first + 2)),
        observed_service_ns: Some(ns),
        stage_ns: [Some(ns), Some(0), Some(0), Some(0), Some(0), Some(0), None],
        observed_stages: 0x3f,
        closed_stages: 0x7f,
        not_applicable_stages: 0x40,
        edge_to_publication_ns: Some(ns),
        edge_to_completion_ns: Some(ns),
        fault_bits: 0,
    }
}

fn census(samples: &[HourlyServiceSampleV1]) -> HourlyServiceCensusV1 {
    HourlyServiceCensusV1 {
        last_batch_id: 1,
        offered_requests: samples.len() as u64,
        admitted_requests: samples.len() as u64,
        closed_requests: samples.len() as u64,
        queued_measurements: samples.len() as u64,
        complete_measurements: samples.len() as u64,
        partial_requests: samples
            .iter()
            .filter(|sample| sample.outcome == HourlyServiceOutcomeV1::Partial)
            .count() as u64,
        refused_requests: samples
            .iter()
            .filter(|sample| sample.outcome == HourlyServiceOutcomeV1::Refused)
            .count() as u64,
        aborted_requests: samples
            .iter()
            .filter(|sample| sample.outcome == HourlyServiceOutcomeV1::Aborted)
            .count() as u64,
        over_limit_measurements: samples
            .iter()
            .filter(|sample| {
                sample
                    .observed_service_ns
                    .is_some_and(|ns| ns > SERVICE_LIMIT_NS)
            })
            .count() as u64,
        total_service_ns: samples
            .iter()
            .filter_map(|sample| sample.observed_service_ns)
            .sum(),
        maximum_service_ns: samples
            .iter()
            .filter_map(|sample| sample.observed_service_ns)
            .max(),
        sealed: true,
        ..HourlyServiceCensusV1::default()
    }
}

fn frame(sequence: u64, kind: MessageKind, payload: Vec<u8>) -> ControlFrame {
    ControlFrame {
        session_id: SessionId::from_bytes([0x51; 16]).unwrap(),
        sequence,
        kind,
        payload,
    }
}

fn record(sequence: u64, hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"H4CR");
    bytes.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    bytes.extend_from_slice(&sequence.to_le_bytes());
    bytes.extend_from_slice(&hour.to_le_bytes());
    bytes.extend_from_slice(&domain.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn data_frame(hour: i32, first: u64, flags: u32) -> Vec<u8> {
    let mut bytes = record(
        first,
        hour,
        53,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &[0; 40],
    );
    let mut end = [0; 32];
    end[16..20].copy_from_slice(&1u32.to_le_bytes());
    end[20..24].copy_from_slice(&1u32.to_le_bytes());
    end[24..28].copy_from_slice(&flags.to_le_bytes());
    bytes.extend(record(
        first + 1,
        hour,
        1000,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
        &end,
    ));
    bytes
}

fn write_receipt(
    path: &Path,
    samples: &[HourlyServiceSampleV1],
    census: Option<HourlyServiceCensusV1>,
) {
    let mut receiver = HourlyServiceReceiver::new(path, binding());
    let batch = HourlyServiceBatchV1 {
        batch_id: 1,
        samples: samples.to_vec(),
    };
    let ack = receiver
        .receive(&frame(
            7,
            MessageKind::HourlyServiceBatch,
            batch.encode().unwrap(),
        ))
        .unwrap();
    assert_eq!(ack.accepted_count, samples.len() as u16);
    if let Some(census) = census {
        assert_eq!(
            receiver
                .receive(&frame(
                    8,
                    MessageKind::HourlyServiceCensus,
                    census.encode().unwrap()
                ))
                .unwrap()
                .accepted_count,
            1
        );
    }
}

fn report(
    samples: &[HourlyServiceSampleV1],
    terminal: Option<HourlyServiceCensusV1>,
    bytes: &[u8],
) -> Value {
    let directory = TestDirectory::new();
    let receipt = directory.0.join(RECEIPT_FILE_NAME);
    let raw = directory.0.join("stream.bin");
    write_receipt(&receipt, samples, terminal);
    std::fs::write(&raw, bytes).unwrap();
    report_for_capture(&receipt, &binding(), &[(0, raw)], true)
}

#[test]
fn final_hour_is_certified_without_a_later_data_frame() {
    let samples = [sample(100, 1, SERVICE_LIMIT_NS)];
    for terminal_lifecycle_ns in [None, Some(0), Some(25_000_000)] {
        let mut terminal = census(&samples);
        terminal.terminal_lifecycle_ns = terminal_lifecycle_ns;
        let value = report(&samples, Some(terminal), &data_frame(100, 1, 0));
        assert_eq!(value["passes_16ms"], true, "{value}");
        assert_eq!(value["service"]["max_ns"], SERVICE_LIMIT_NS);
        assert_eq!(value["terminal_lifecycle_ns"], json!(terminal_lifecycle_ns));
        assert_eq!(
            value["census"]["terminal_lifecycle_ns"],
            json!(terminal_lifecycle_ns)
        );
    }
}

#[test]
fn unstarted_teardown_boundary_is_reported_without_faulting_the_population() {
    let samples = [sample(100, 1, 1)];
    let mut terminal = census(&samples);
    terminal.boundary_excluded_hours = 1;
    let value = report(&samples, Some(terminal), &data_frame(100, 1, 0));
    assert_eq!(value["schema"], "hoi4.hourly_service.v2");
    assert_eq!(value["timing_population_complete"], true, "{value}");
    assert_eq!(value["census"]["unmeasured_hours"], 0);
    assert_eq!(value["census"]["boundary_excluded_hours"], 1);
}

#[test]
fn one_nanosecond_over_limit_is_not_rounded_down() {
    let samples = [sample(100, 1, SERVICE_LIMIT_NS + 1)];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 1, 0));
    assert_eq!(value["timing_population_complete"], true, "{value}");
    assert_eq!(value["passes_16ms"], false);
    assert_eq!(value["service"]["max_us_ceil"], 16001);
    assert_eq!(value["service"]["over_16ms_count"], 1);
}

#[test]
fn missing_terminal_census_never_certifies_the_last_tail() {
    let samples = [sample(100, 1, 1)];
    let value = report(&samples, None, &data_frame(100, 1, 0));
    assert_eq!(value["timing_population_complete"], false);
    assert_eq!(value["passes_16ms"], false);
}

#[test]
fn delayed_closures_are_joined_by_original_owner_not_arrival_order() {
    let samples = [sample(101, 3, 2), sample(100, 1, 1)];
    let mut bytes = data_frame(100, 1, 0);
    bytes.extend(data_frame(101, 3, 0));
    let value = report(&samples, Some(census(&samples)), &bytes);
    assert_eq!(value["passes_16ms"], true, "{value}");
    assert_eq!(value["raw_joined_samples"], 2);
}

#[test]
fn a_claimed_subrange_cannot_hide_the_start_of_a_data_frame() {
    let mut item = sample(100, 1, 1);
    item.data_sequence = Some((2, 3));
    let samples = [item];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 1, 0));
    assert_eq!(value["timing_population_complete"], false);
}

#[test]
fn unknown_stream_and_duplicate_owner_do_not_pass() {
    let mut item = sample(100, 1, 1);
    item.owner.logical_stream_id = 1;
    let samples = [item, item];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 1, 0));
    assert_eq!(value["timing_population_complete"], false);
    assert_eq!(value["passes_16ms"], false);
}

#[test]
fn missing_closed_stage_is_not_a_measured_zero() {
    let mut item = sample(100, 1, 1);
    item.closed_stages &= !2;
    let samples = [item];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 1, 0));
    assert_eq!(value["timing_population_complete"], false);
}

#[test]
fn fully_timed_partial_frame_stays_in_tails_without_claiming_complete_data() {
    let mut item = sample(100, 1, SERVICE_LIMIT_NS + 1);
    item.outcome = HourlyServiceOutcomeV1::Partial;
    let samples = [item];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 1, 1));
    assert_eq!(value["timing_population_complete"], true, "{value}");
    assert_eq!(value["complete_capture"], false);
    assert_eq!(value["service"]["over_16ms_count"], 1);
}

#[test]
fn explicitly_unexecuted_refusal_stages_are_not_fake_zeroes() {
    let mut item = sample(100, 1, 2);
    item.outcome = HourlyServiceOutcomeV1::Refused;
    item.data_sequence = None;
    item.stage_ns = [Some(2), None, None, None, None, None, None];
    item.observed_stages = 1;
    item.not_applicable_stages = 0x7e;
    item.edge_to_publication_ns = None;
    let mut problems = Vec::new();
    let values = service_values(&[item], &mut problems);
    validate_census(&census(&[item]), &[item], &mut problems);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(values, [2]);
}

#[test]
fn gap_only_footer_is_reported_and_never_becomes_a_cheap_complete_hour() {
    let samples = [sample(100, 1, 1)];
    let mut bytes = data_frame(100, 1, 0);
    bytes.extend(record(
        3,
        101,
        1000,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
        &[0; 32],
    ));
    let value = report(&samples, Some(census(&samples)), &bytes);
    assert_eq!(value["raw_gap_only_frames"], 1);
    assert_eq!(value["complete_capture"], false);
    assert_eq!(value["passes_16ms"], false);
    assert_eq!(value["service"]["count"], 1);
}

#[test]
fn census_losses_unattributed_work_and_pending_delivery_each_invalidate_coverage() {
    let samples = [sample(100, 1, 1)];
    for fault in 0..4 {
        let mut terminal = census(&samples);
        match fault {
            0 => terminal.lost_measurements = 1,
            1 => terminal.unattributed_service_ns = 1,
            2 => terminal.pending_delivery = 1,
            _ => terminal.total_service_ns = 0,
        }
        let value = report(&samples, Some(terminal), &data_frame(100, 1, 0));
        assert_eq!(value["timing_population_complete"], false, "{value}");
    }
}

#[test]
fn stage_union_overflow_and_impossible_total_are_rejected() {
    let mut item = sample(100, 1, 1);
    item.stage_ns[1] = Some(u64::MAX);
    let mut problems = Vec::new();
    service_values(&[item], &mut problems);
    assert!(!problems.is_empty());
    item = sample(100, 1, 1);
    item.observed_service_ns = Some(2);
    problems.clear();
    service_values(&[item], &mut problems);
    assert!(!problems.is_empty());
}

#[test]
fn receiver_acknowledges_memory_custody_before_any_filesystem_work() {
    let directory = TestDirectory::new();
    let missing_parent = directory.0.join("absent").join(RECEIPT_FILE_NAME);
    let mut receiver = HourlyServiceReceiver::new(&missing_parent, binding());
    let batch = HourlyServiceBatchV1 {
        batch_id: 1,
        samples: vec![sample(100, 1, 1)],
    };
    assert_eq!(
        receiver
            .receive(&frame(
                7,
                MessageKind::HourlyServiceBatch,
                batch.encode().unwrap()
            ))
            .unwrap()
            .accepted_count,
        1
    );
    assert!(!missing_parent.exists());
    drop(receiver);
    assert!(!missing_parent.exists());
    assert!(load_receipt(&missing_parent, &binding()).census.is_none());
}

#[test]
fn foreign_authenticated_session_is_retained_but_never_acknowledged_as_valid() {
    let directory = TestDirectory::new();
    let path = directory.0.join(RECEIPT_FILE_NAME);
    let mut receiver = HourlyServiceReceiver::new(&path, binding());
    let batch = HourlyServiceBatchV1 {
        batch_id: 1,
        samples: vec![sample(100, 1, 1)],
    };
    let mut foreign = frame(7, MessageKind::HourlyServiceBatch, batch.encode().unwrap());
    foreign.session_id = SessionId::from_bytes([0x52; 16]).unwrap();
    assert_eq!(receiver.receive(&foreign).unwrap().accepted_count, 0);
    drop(receiver);
    assert!(!load_receipt(&path, &binding()).problems.is_empty());
}

#[test]
fn custody_marker_and_exact_target_binding_are_required() {
    let directory = TestDirectory::new();
    let path = directory.0.join(RECEIPT_FILE_NAME);
    let samples = [sample(100, 1, 1)];
    write_receipt(&path, &samples, Some(census(&samples)));
    let mut other = binding();
    other.process_start_nonce += 1;
    assert!(!load_receipt(&path, &other).problems.is_empty());
    let bytes = std::fs::read(&path).unwrap();
    let end = bytes[..bytes.len() - 1]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .unwrap()
        + 1;
    std::fs::write(&path, &bytes[..end]).unwrap();
    assert!(!load_receipt(&path, &binding()).problems.is_empty());
}

#[test]
fn direct_capture_without_control_receipt_reports_unavailable_without_reading_data() {
    let directory = TestDirectory::new();
    let value = report_for_capture(
        &directory.0.join(RECEIPT_FILE_NAME),
        &binding(),
        &[(0, directory.0.join("also-absent.bin"))],
        true,
    );
    assert_eq!(value["available"], false);
    assert_eq!(value["passes_16ms"], false);
}

#[test]
fn preselection_service_and_optional_edge_latencies_keep_their_distinct_origins() {
    let mut item = sample(100, 1, SERVICE_LIMIT_NS);
    item.edge_to_publication_ns = Some(1);
    item.edge_to_completion_ns = Some(1);
    let samples = [item];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 1, 0));
    assert_eq!(value["passes_16ms"], true, "{value}");
    item.edge_to_publication_ns = None;
    item.edge_to_completion_ns = None;
    let samples = [item];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 1, 0));
    assert_eq!(value["timing_population_complete"], true, "{value}");
    assert_eq!(value["edge_to_completion"]["count"], 0);
    assert_eq!(value["edge_to_completion"]["max_ns"], Value::Null);
}

#[test]
fn authenticated_identity_origin_and_freeze_preamble_do_not_shift_data_ownership() {
    use compact_contract::campaign_identity::{
        encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
    };
    use compact_contract::freeze_census_record::{
        FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
    };
    let hour = compact_contract::calendar::DATE_EPOCH_CONST + 1944 * 365 * 24;
    let observation = Cid1Observation {
        game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".into(),
        game_unique_seed: -947102,
        game_hour_raw: hour,
    };
    let fields = Cid1Fields::from_stable_observations(
        observation.clone(),
        observation,
        "test_build".into(),
        Cid1Witnesses::default(),
    )
    .unwrap();
    let identity = encode_cid1(&fields).unwrap();
    let install = FreezeCensusRecordV1::encode_bounded(
        FreezeCensusPhase::Install,
        FreezeCensusCounterSnapshot::default(),
        FreezeCensusCounterSnapshot::default(),
        &[],
    )
    .unwrap();
    let release = FreezeCensusRecordV1::encode_bounded(
        FreezeCensusPhase::Release,
        FreezeCensusCounterSnapshot::default(),
        FreezeCensusCounterSnapshot::default(),
        &[],
    )
    .unwrap();
    let mut bytes = record(
        0,
        hour,
        COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
        &identity,
    );
    bytes.extend(record(
        1,
        0,
        1000,
        compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
        &install,
    ));
    bytes.extend(data_frame(hour, 2, 0));
    bytes.extend(record(
        4,
        0,
        1000,
        compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
        &release,
    ));
    let samples = [sample(hour, 2, 1)];
    let value = report(&samples, Some(census(&samples)), &bytes);
    assert_eq!(value["passes_16ms"], true, "{value}");
    assert_eq!(value["raw_joined_samples"], 1);
    bytes[20..22].copy_from_slice(&53u16.to_le_bytes());
    let value = report(&samples, Some(census(&samples)), &bytes);
    assert_eq!(value["timing_population_complete"], false);
}

#[test]
fn a_sequence_zero_reader_record_cannot_impersonate_an_identity_preamble() {
    let samples = [sample(100, 0, 1)];
    let value = report(&samples, Some(census(&samples)), &data_frame(100, 0, 0));
    assert_eq!(value["timing_population_complete"], false);
}

#[test]
fn host_custody_capacity_refuses_extra_samples_without_losing_the_retained_prefix() {
    let directory = TestDirectory::new();
    let path = directory.0.join(RECEIPT_FILE_NAME);
    let mut receiver = HourlyServiceReceiver::new(&path, binding());
    for batch_id in 1..=256u64 {
        let batch = HourlyServiceBatchV1 {
            batch_id,
            samples: vec![sample(100, 1, 1); 16],
        };
        assert_eq!(
            receiver
                .receive(&frame(
                    batch_id,
                    MessageKind::HourlyServiceBatch,
                    batch.encode().unwrap()
                ))
                .unwrap()
                .accepted_count,
            16
        );
    }
    let extra = HourlyServiceBatchV1 {
        batch_id: 257,
        samples: vec![sample(101, 3, 1)],
    };
    assert_eq!(
        receiver
            .receive(&frame(
                257,
                MessageKind::HourlyServiceBatch,
                extra.encode().unwrap()
            ))
            .unwrap()
            .accepted_count,
        0
    );
    assert_eq!(receiver.samples, SAMPLE_LIMIT);
    assert!(receiver.failed);
    drop(receiver);
    let receipt = load_receipt(&path, &binding());
    assert_eq!(receipt.samples.len(), SAMPLE_LIMIT);
    assert!(!receipt.problems.is_empty());
}

#[test]
fn faulted_hour_costs_remain_visible_without_false_census_mismatch() {
    for fault_bits in [512, 520] {
        for outcome in [
            HourlyServiceOutcomeV1::Complete,
            HourlyServiceOutcomeV1::Partial,
        ] {
            for has_complete_measurement in [false, true] {
                let slow = SERVICE_LIMIT_NS + 19;
                let mut samples = Vec::new();
                let mut bytes = Vec::new();
                if has_complete_measurement {
                    samples.push(sample(100, 1, 1));
                    bytes.extend(data_frame(100, 1, 0));
                }
                let hour = 100 + i32::from(has_complete_measurement);
                let first = 1 + 2 * u64::from(has_complete_measurement);
                let mut failed = sample(hour, first, slow);
                failed.outcome = outcome;
                failed.fault_bits = fault_bits;
                if fault_bits == 520 {
                    failed.stage_ns = [Some(slow), Some(0), None, None, None, None, None];
                    failed.observed_stages = 3;
                    failed.closed_stages = 0x43;
                }
                samples.push(failed);
                bytes.extend(data_frame(
                    hour,
                    first,
                    u32::from(outcome == HourlyServiceOutcomeV1::Partial),
                ));
                let complete = u64::from(has_complete_measurement);
                let terminal = HourlyServiceCensusV1 {
                    last_batch_id: 1,
                    offered_requests: complete + 1,
                    admitted_requests: complete + 1,
                    closed_requests: complete + 1,
                    queued_measurements: complete + 1,
                    complete_measurements: complete,
                    incomplete_measurements: 1,
                    partial_requests: u64::from(outcome == HourlyServiceOutcomeV1::Partial),
                    total_service_ns: complete,
                    maximum_service_ns: has_complete_measurement.then_some(1),
                    fault_bits,
                    sealed: true,
                    ..HourlyServiceCensusV1::default()
                };
                let value = report(&samples, Some(terminal), &bytes);
                assert_eq!(value["service"]["count"], complete + 1, "{value}");
                assert_eq!(value["service"]["total_ns"], slow + complete);
                assert_eq!(value["service"]["max_ns"], slow);
                assert_eq!(value["service"]["over_16ms_count"], 1);
                assert_eq!(value["timing_population_complete"], false);
                assert_eq!(value["timing_within_16ms"], false);
                assert_eq!(value["passes_16ms"], false);
                assert!(value["problems"].as_array().unwrap().iter().all(|problem|
                    problem != "terminal census totals or outcome counts differ from the raw received samples"), "{value}");
            }
        }
    }
}

#[test]
fn faulted_population_does_not_hide_tampered_complete_census_fields() {
    let good = sample(100, 1, 1);
    let mut failed = sample(101, 3, SERVICE_LIMIT_NS + 1);
    failed.fault_bits = 512;
    let samples = [good, failed];
    let mut bytes = data_frame(100, 1, 0);
    bytes.extend(data_frame(101, 3, 0));
    for field in 0..5 {
        let mut terminal = census(&[good]);
        terminal.offered_requests = 2;
        terminal.admitted_requests = 2;
        terminal.closed_requests = 2;
        terminal.queued_measurements = 2;
        terminal.incomplete_measurements = 1;
        terminal.fault_bits = 512;
        match field {
            0 => terminal.total_service_ns += 1,
            1 => terminal.maximum_service_ns = Some(2),
            2 => terminal.over_limit_measurements = 1,
            3 => terminal.complete_measurements = 2,
            _ => terminal.incomplete_measurements = 0,
        }
        let value = report(&samples, Some(terminal), &bytes);
        assert_eq!(value["passes_16ms"], false);
        assert!(value["problems"].as_array().unwrap().iter().any(|problem|
            problem == "terminal census totals or outcome counts differ from the raw received samples"), "{field}: {value}");
        assert_eq!(value["service"]["total_ns"], SERVICE_LIMIT_NS + 2);
    }
}
