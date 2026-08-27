use super::*;

fn fixture_diagnosis(reason: &str) -> compact_contract::frame_degraded::FrameDegradedDiagnosis {
    compact_contract::frame_degraded::FrameDegradedDiagnosis::undeclared(
        compact_contract::diagnostic_channel::Authority::SelfDomain,
        reason,
    )
}
use compact_contract::calendar::DATE_EPOCH_CONST;
use compact_contract::campaign_identity::{
    encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
    COMPACT_RECORD_TYPE_STREAM_HEADER,
};
use compact_contract::{
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION, COMPACT_STATUS_DELTA_DATA,
    COMPACT_STATUS_NO_CLOCK, COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_SOURCE_UNAVAILABLE,
    COMPACT_STATUS_WORKER_PANIC,
};
use std::io::Cursor;
use std::path::PathBuf;

struct TmpDir(PathBuf);
impl TmpDir {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "hoi4_fold_check_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn raw_for(hour_index: i64) -> i32 {
    DATE_EPOCH_CONST + hour_index as i32
}

fn push_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    push_record_with_flags(
        out,
        sequence,
        game_hour_raw,
        domain_id,
        record_type,
        0,
        payload,
    );
}

fn push_record_with_flags(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&record_flags.to_le_bytes());
    out.extend_from_slice(payload);
}

fn country_identity_payload(rows: &[(u32, &str)]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    for (tag_db_id, tag) in rows {
        p.extend_from_slice(&tag_db_id.to_le_bytes());
        p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        p.extend_from_slice(tag.as_bytes());
        p.push(1);
    }
    p
}

fn perf_payload() -> Vec<u8> {
    perf_payload_with_status(0)
}

fn frame_end_payload() -> Vec<u8> {
    frame_end_payload_with(1, 1, 0)
}

fn empty_frame_end_payload() -> Vec<u8> {
    frame_end_payload_with(0, 0, 0)
}

fn perf_payload_with_status(status_flags: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 40];
    payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
    payload
}

fn frame_end_payload_with(emitted: u32, scheduled: u32, status_flags: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 32];
    payload[16..20].copy_from_slice(&emitted.to_le_bytes());
    payload[20..24].copy_from_slice(&scheduled.to_le_bytes());
    payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
    payload
}

const DOMAIN_FRAME: u16 = 1000;

const HOUR_A: i64 = 16_959_360;
const HOUR_B: i64 = 16_959_361;
const HOUR_C: i64 = 16_959_362;

fn synthetic_stream() -> (Vec<u8>, usize) {
    let mut s = Vec::new();
    let mut seq = 1u64;

    let a_payload = country_identity_payload(&[(47, "GER")]);
    let mutation_offset = s.len() + COMPACT_RECORD_HEADER_LEN + 20;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &a_payload,
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    push_record(
        &mut s,
        seq,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    push_record(
        &mut s,
        seq,
        raw_for(HOUR_C),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER"), (48, "FRA")]),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_C),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_C),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    (s, mutation_offset)
}

fn identity_v1_stream(ordinary: &[u8]) -> Vec<u8> {
    let observation = Cid1Observation {
        game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
        game_unique_seed: -947_102,
        game_hour_raw: raw_for(HOUR_A),
    };
    let fields = Cid1Fields::from_stable_observations(
        observation.clone(),
        observation,
        "vanilla_latest_2026_07_15".to_string(),
        Cid1Witnesses::default(),
    )
    .unwrap();
    let payload = encode_cid1(&fields).unwrap();
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        0,
        raw_for(HOUR_A),
        COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
        &payload,
    );
    stream.extend_from_slice(ordinary);
    stream
}

fn fold_bytes(stream: &[u8]) -> FoldRun {
    collect_run(Cursor::new(stream.to_vec()), stream.len() as u64).expect("fold synthetic stream")
}

fn clock_unreadable_gap_payload(hour_index: i64, outcome_ordinal: u64) -> Vec<u8> {
    compact_contract::domain_gap::DomainGapRecordV1 {
        part: compact_contract::domain_gap::DomainGapPart::State,
        reason: compact_contract::domain_gap::DomainGapReason::ClockUnreadable,
        target_hour_raw: raw_for(hour_index),
        range_start_hour_raw: raw_for(hour_index),
        range_end_hour_raw: raw_for(hour_index),
        tick_before: 0,
        tick_after: 0,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal,
    }
    .encode()
    .expect("closed ClockUnreadable gap witnesses")
}

fn worker_panic_gap_payload(hour_index: i64, outcome_ordinal: u64) -> Vec<u8> {
    compact_contract::domain_gap::DomainGapRecordV1 {
        part: compact_contract::domain_gap::DomainGapPart::State,
        reason: compact_contract::domain_gap::DomainGapReason::WorkerPanic,
        target_hour_raw: raw_for(hour_index),
        range_start_hour_raw: raw_for(hour_index),
        range_end_hour_raw: raw_for(hour_index),
        tick_before: 103,
        tick_after: 103,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal,
    }
    .encode()
    .expect("closed WorkerPanic gap witnesses")
}

fn refusal_payload(hour_index: i64, domain_id: u16) -> Vec<u8> {
    use compact_contract::diagnostic_channel::{
        Authority, DiagUnit, LocatorLevel, Predicate, TruncationAccount,
    };
    compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: raw_for(hour_index),
        offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
            domain_id,
            status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            predicate: Predicate::new(0x0002_0001, "synthetic_source_slot_is_bound").unwrap(),
            authority: Authority::OtherDomain(91),
            locator: vec![LocatorLevel::new("region", 41, Some(96))],
            units: vec![DiagUnit::deciding("synthetic_offender", 1)],
            truncation: TruncationAccount::complete(1),
            off_frame: None,
            reason: "synthetic_offender=1".to_string(),
            reason_units_named: 1,
        }],
        frame_findings: Vec::new(),
    }
    .encode()
    .expect("a version 2 refusal names an offender")
}

fn frame_only_refusal_payload(hour_index: i64) -> Vec<u8> {
    use compact_contract::diagnostic_channel::{Authority, Predicate};
    compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: raw_for(hour_index),
        offenders: Vec::new(),
        frame_findings: vec![compact_contract::frame_refusal::FrameRefusalFinding {
            predicate: Predicate::new(0x0001_0003, "frame_clock_is_readable").unwrap(),
            authority: Authority::Host,
            locator: Vec::new(),
            reason: "framing: the frame's own clock could not be read".to_string(),
        }],
    }
    .encode()
    .expect("a frame-level-only refusal is a complete record")
}

fn refusal_payload_at_version(hour_index: i64, domain_id: u16, version: u16) -> Vec<u8> {
    let mut wire = refusal_payload(hour_index, domain_id);
    wire[4..6].copy_from_slice(&version.to_le_bytes());
    wire
}

fn one_domain_roster() -> ExpectedRoster {
    ExpectedRoster::test(&[(18, true)])
}

#[test]
fn folds_all_hours() {
    let (stream, _) = synthetic_stream();
    let run = fold_bytes(&stream);
    assert_eq!(run.hours.len(), 3, "one snapshot per game-hour");
    assert_eq!(run.totals.hours_emitted, 3);
    assert_eq!(run.hours[0].hour_index, HOUR_A);
    assert_eq!(run.hours[2].hour_index, HOUR_C);
    assert_eq!(
        run.totals.truncated_tail_bytes, 0,
        "clean stream has no tail"
    );
    assert!(run.totals.stop_reason.is_none());
}

#[test]
fn identity_v1_header_is_consumed_before_completeness_and_fold() {
    let (ordinary, _) = synthetic_stream();
    let identity_stream = identity_v1_stream(&ordinary);
    let identity_run = collect_identity_v1_run(
        Cursor::new(identity_stream.clone()),
        identity_stream.len() as u64,
    )
    .expect("fold identity-v1 stream");
    let legacy_run = fold_bytes(&ordinary);

    assert_eq!(identity_run.hours.len(), legacy_run.hours.len());
    assert_eq!(
        chain_hash(&identity_run.hours),
        chain_hash(&legacy_run.hours)
    );
    assert_eq!(
        identity_run.totals.data_records,
        legacy_run.totals.data_records
    );
    assert_eq!(
        identity_run.totals.perf_records,
        legacy_run.totals.perf_records
    );
    assert_eq!(
        identity_run.totals.frame_end_records,
        legacy_run.totals.frame_end_records
    );
    assert_eq!(identity_run.totals.other_records, 0);
}

#[test]
fn identity_v1_mode_refuses_an_ordinary_record_before_cid1() {
    let (ordinary, _) = synthetic_stream();
    let error = match collect_identity_v1_run(Cursor::new(ordinary.clone()), ordinary.len() as u64)
    {
        Ok(_) => panic!("identity-v1 must not fall back to a legacy stream"),
        Err(error) => error,
    };
    assert!(
        error.contains("identity-v1 stream validation at byte 0"),
        "{error}"
    );
    assert!(error.contains("invalid CID1 stream header"), "{error}");
}

#[test]
fn identity_mode_cli_is_explicit_and_defaults_to_legacy() {
    assert_eq!(
        parse_stream_identity_mode(&[]).unwrap(),
        StreamIdentityMode::Legacy
    );
    assert_eq!(
        parse_stream_identity_mode(&["--identity-mode".to_string(), "identity-v1".to_string(),])
            .unwrap(),
        StreamIdentityMode::IdentityV1
    );
    assert!(
        parse_stream_identity_mode(&["--identity-mode".to_string(), "auto".to_string(),]).is_err()
    );
}

#[test]
fn hashing_is_stable_across_runs() {
    let (stream, _) = synthetic_stream();
    let a = fold_bytes(&stream);
    let b = fold_bytes(&stream);
    assert_eq!(chain_hash(&a.hours), chain_hash(&b.hours));
    for (x, y) in a.hours.iter().zip(b.hours.iter()) {
        assert_eq!(x.hour_index, y.hour_index);
        assert_eq!(
            x.snapshot_hash, y.snapshot_hash,
            "hour {} hash unstable",
            x.hour_index
        );
    }
}

#[test]
fn perf_payload_is_ignored_by_the_fold_but_presence_is_completeness() {
    let (with_perf, _) = synthetic_stream();
    let run_with = fold_bytes(&with_perf);
    assert_eq!(run_with.totals.perf_records, 3, "PERF records were seen");

    let mut s = Vec::new();
    let mut seq = 1u64;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &empty_frame_end_payload(),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_C),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER"), (48, "FRA")]),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_C),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    seq += 1;
    push_record(
        &mut s,
        seq,
        raw_for(HOUR_C),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let run_without = fold_bytes(&s);

    assert_eq!(run_without.totals.perf_records, 2);
    for (with, without) in run_with.hours.iter().zip(&run_without.hours) {
        assert_eq!(
            with.snapshot_hash, without.snapshot_hash,
            "PERF must not change folded state"
        );
    }
    assert_ne!(
        chain_hash(&run_with.hours),
        chain_hash(&run_without.hours),
        "losing authoritative PERF completeness must change G1 identity"
    );
}

#[test]
fn normalized_perf_status_changes_identity_even_when_snapshots_match() {
    fn stream(status_flags: u32) -> Vec<u8> {
        let mut out = Vec::new();
        push_record(
            &mut out,
            1,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut out,
            2,
            raw_for(HOUR_A),
            26,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(status_flags),
        );
        push_record(
            &mut out,
            3,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        out
    }

    let clean = fold_bytes(&stream(0));
    let expected_partial = fold_bytes(&stream(COMPACT_STATUS_PARTIAL_DATA));
    assert_eq!(
        clean.hours[0].snapshot_hash, expected_partial.hours[0].snapshot_hash,
        "PERF status is not folded state"
    );
    assert_ne!(
        clean.hours[0].completeness,
        expected_partial.hours[0].completeness
    );
    assert_ne!(
        chain_hash(&clean.hours),
        chain_hash(&expected_partial.hours)
    );
}

#[test]
fn forged_two_of_two_frame_counts_do_not_certify_one_observed_domain() {
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut stream,
        2,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    push_record(
        &mut stream,
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(2, 2, 0),
    );

    let expected = ExpectedRoster::test(&[(18, true), (26, true)]);
    let err = collect_certified_run(Cursor::new(stream.clone()), stream.len() as u64, &expected)
        .err()
        .expect("self-consistent FRAME_END counts must not define coverage");
    assert!(err.contains("domain division_equipment (26) PERF"), "{err}");
    assert!(err.contains("observed 0"), "{err}");
}

#[test]
fn expected_profile_is_mandatory_and_only_production_is_authoritative() {
    let missing = parse_expected_profile(&[]).unwrap_err();
    assert!(missing.contains("mandatory --expected-profile production"));

    let unsupported =
        parse_expected_profile(&["--expected-profile".to_string(), "debug".to_string()])
            .unwrap_err();
    assert!(unsupported.contains("only \"production\" is authoritative"));

    let production =
        parse_expected_profile(&["--expected-profile".to_string(), "production".to_string()])
            .expect("production roster");
    assert_eq!(production.profile, "production");
    assert_eq!(
        production.domains.len(),
        compact_contract::passive_capture_roster().len()
    );
    assert!(production.domains.contains_key(&18));

    for id in compact_contract::PASSIVE_CAPTURE_ONLY_DOMAIN_IDS {
        assert!(
            !production.domains[id].mandatory,
            "scheduled-only domain {id} must not be mandatory"
        );
    }
    assert!(production.domains[&18].mandatory);
    assert_eq!(
        production.domains.values().filter(|d| d.mandatory).count(),
        compact_contract::production_profile_domains()
            .into_iter()
            .filter(|d| compact_contract::passive_production_admits(d.id))
            .count()
    );
}

#[test]
fn dropped_unchanged_data_record_changes_identity_not_snapshot() {
    fn stream(data_records: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let payload = country_identity_payload(&[(47, "GER")]);
        let mut sequence = 1u64;
        for _ in 0..data_records {
            push_record(
                &mut out,
                sequence,
                raw_for(HOUR_A),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            sequence += 1;
        }
        push_record(
            &mut out,
            sequence,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(),
        );
        sequence += 1;
        push_record(
            &mut out,
            sequence,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        out
    }

    let repeated = fold_bytes(&stream(2));
    let dropped = fold_bytes(&stream(1));
    assert_eq!(
        repeated.hours[0].snapshot_hash, dropped.hours[0].snapshot_hash,
        "dropping an unchanged full-replace record must leave folded state equal"
    );
    assert_eq!(repeated.hours[0].completeness.domains[&18].data_records, 2);
    assert_eq!(dropped.hours[0].completeness.domains[&18].data_records, 1);
    assert_ne!(chain_hash(&repeated.hours), chain_hash(&dropped.hours));
}

#[test]
fn non_tolerated_perf_failure_cannot_certify() {
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut stream,
        2,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload_with_status(COMPACT_STATUS_SOURCE_UNAVAILABLE),
    );
    push_record(
        &mut stream,
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let err = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .err()
        .expect("non-tolerated PERF failure must fail certification");
    assert!(err.contains("non-tolerated failure flags"), "{err}");
}

#[test]
fn partial_tolerance_is_not_granted_to_unlisted_domains() {
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut stream,
        2,
        raw_for(HOUR_A),
        29,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload_with_status(COMPACT_STATUS_PARTIAL_DATA),
    );
    push_record(
        &mut stream,
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let err = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .err()
        .expect("unlisted partial domain must fail certification");
    assert!(err.contains("domain 29 PERF status"), "{err}");
    assert!(err.contains("non-tolerated failure flags"), "{err}");
}

#[test]
fn compare_catches_a_single_mutated_payload_byte() {
    let (stream, offset) = synthetic_stream();
    let baseline = fold_bytes(&stream);

    let mut mutated = stream.clone();
    mutated[offset] ^= 0x01;
    let after = fold_bytes(&mutated);

    assert_eq!(baseline.hours.len(), after.hours.len());
    assert_ne!(
        chain_hash(&baseline.hours),
        chain_hash(&after.hours),
        "a single mutated payload byte must change the chain hash"
    );
    assert_ne!(
        baseline.hours[0].snapshot_hash, after.hours[0].snapshot_hash,
        "hour A (mutated) hash must differ"
    );
    assert_eq!(
        baseline.hours[2].snapshot_hash, after.hours[2].snapshot_hash,
        "hour C (untouched) hash must match"
    );
}

#[test]
fn truncated_tail_is_rejected_even_when_complete_prefix_hours_exist() {
    let (stream, _) = synthetic_stream();
    let truncated = &stream[..stream.len() - 5];
    let err = match collect_run(Cursor::new(truncated.to_vec()), truncated.len() as u64) {
        Ok(_) => panic!("fold_check must reject a truncated stream"),
        Err(err) => err,
    };
    assert!(err.contains("truncated/corrupt tail"), "{err}");
    assert!(err.contains("stream boundary is not closed"), "{err}");
}

#[test]
fn malformed_domain_payload_is_excluded_at_a_closed_hour_boundary() {
    let mut stream = Vec::new();
    let mut malformed = country_identity_payload(&[(47, "GER")]);
    malformed.push(0xAA);
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &malformed,
    );
    push_record(
        &mut stream,
        2,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &empty_frame_end_payload(),
    );
    let run = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .expect("exact-invalid DOMAIN_DATA is a recorded exclusion, not a structural refusal");
    assert_eq!(run.totals.malformed_payload_records, 1);
    assert_eq!(run.totals.excluded_domain_data_records.len(), 1);
    let excluded = &run.totals.excluded_domain_data_records[0];
    assert_eq!(excluded.stream_offset, 0);
    assert_eq!(excluded.game_hour_raw, raw_for(HOUR_A));
    assert_eq!(excluded.domain_id, 18);
    assert!(!excluded.validation_reason.is_empty(), "{excluded:?}");
}

#[test]
fn open_hour_eof_is_rejected_without_relying_on_tail_bytes() {
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    let err = match collect_run(Cursor::new(stream.clone()), stream.len() as u64) {
        Ok(_) => panic!("fold_check must reject an open hour at EOF"),
        Err(err) => err,
    };
    assert!(err.contains("left open at EOF"), "{err}");
}

#[test]
fn fold_check_rejects_misplaced_flags_before_data_decode() {
    let mut stream = Vec::new();
    push_record_with_flags(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut stream,
        2,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &empty_frame_end_payload(),
    );
    let err = match collect_run(Cursor::new(stream.clone()), stream.len() as u64) {
        Ok(_) => panic!("fold_check must reject misplaced record flags"),
        Err(err) => err,
    };
    assert!(err.contains("rejected bits"), "{err}");
}

#[test]
fn fold_check_rejects_noncanonical_sequence_and_final_boundary() {
    let mut wrong_start = Vec::new();
    push_record(
        &mut wrong_start,
        2,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut wrong_start,
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &empty_frame_end_payload(),
    );
    let err = match collect_run(Cursor::new(wrong_start.clone()), wrong_start.len() as u64) {
        Ok(_) => panic!("fold_check must reject a sequence that does not start at one"),
        Err(err) => err,
    };
    assert!(
        err.contains("compact record sequence 2 is not the next required sequence 1"),
        "{err}"
    );

    let mut trailing_perf = Vec::new();
    push_record(
        &mut trailing_perf,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut trailing_perf,
        2,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &empty_frame_end_payload(),
    );
    push_record(
        &mut trailing_perf,
        3,
        raw_for(HOUR_A + 1),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    let err = match collect_run(
        Cursor::new(trailing_perf.clone()),
        trailing_perf.len() as u64,
    ) {
        Ok(_) => panic!("fold_check must reject a stream not ending on FRAME_END"),
        Err(err) => err,
    };
    assert!(
        err.contains("stream is not certifiable"),
        "the terminal-boundary verdict must come from the shared certification authority: {err}"
    );
    assert!(
        err.contains("final record must be a canonical FRAME_END or FRAME_REFUSED boundary"),
        "{err}"
    );
}

#[test]
fn torn_zstd_frame_is_a_hard_fold_check_error() {
    let dir = TmpDir::new("torn_zstd");
    let (stream, _) = synthetic_stream();
    let raw_path = dir.0.join("raw.bin");
    let zstd_path = dir.0.join("compressed.bin");
    std::fs::write(&raw_path, stream).unwrap();
    compact_stream::encode_file(
        &raw_path,
        &zstd_path,
        compact_stream::StreamCompression::Zstd,
    )
    .unwrap();

    let mut stored = std::fs::read(&zstd_path).unwrap();
    stored.pop().expect("checksummed zstd frame is non-empty");
    std::fs::write(&zstd_path, stored).unwrap();

    let err = match run_input(&zstd_path, &ExpectedRoster::production()) {
        Ok(_) => panic!("fold_check must reject a torn zstd frame"),
        Err(err) => err,
    };
    assert!(
        err.contains("incomplete zstd frame"),
        "unexpected error: {err}"
    );
}

#[test]
fn legacy_pre222_container_decodes_for_diagnostics_but_does_not_certify() {
    let path = std::fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../snapshot/tests/data/legacy/pre222_store_deflate.battleplan"),
    )
    .expect("canonicalize checked-in legacy replay fixture");
    let err = match run_input(&path, &ExpectedRoster::production()) {
        Ok(_) => panic!("legacy zero-payload FRAME_END must not pass current certification"),
        Err(err) => err,
    };
    assert!(err.contains("compact domain zero is reserved"), "{err}");
    assert!(
        err.contains("FRAME_END domain 0 is not 1000"),
        "historical envelope should remain diagnosable without becoming certifiable: {err}"
    );
}

#[test]
fn domain_gap_is_the_domains_terminal_outcome_not_a_missing_perf_record() {
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_GAP,
        &clock_unreadable_gap_payload(HOUR_A, 0),
    );
    push_record(
        &mut stream,
        2,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );

    let err = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .err()
    .expect("an accounted absence is still an absence and cannot certify the hour");
    assert!(
        err.contains("terminal outcome is 1 DOMAIN_GAP record(s)"),
        "{err}"
    );
    assert!(err.contains("ClockUnreadable"), "{err}");
    assert!(err.contains("accounted absence"), "{err}");
    assert!(
        !err.contains("PERF record count 1, observed 0"),
        "a gap must not be reported as a lost PERF record: {err}"
    );
}

#[test]
fn why_a_domain_was_absent_is_part_of_the_hours_certified_identity() {
    fn stream(gap_payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        push_record(
            &mut out,
            1,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut out,
            2,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(),
        );
        push_record(
            &mut out,
            3,
            raw_for(HOUR_A),
            26,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            gap_payload,
        );
        push_record(
            &mut out,
            4,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(2, 2, 0),
        );
        out
    }

    let clock = fold_bytes(&stream(&clock_unreadable_gap_payload(HOUR_A, 0)));
    let panic = fold_bytes(&stream(&worker_panic_gap_payload(HOUR_A, 0)));
    assert_eq!(clock.hours.len(), 1);
    assert_eq!(panic.hours.len(), 1);
    assert_eq!(
        clock.hours[0].world_hash, panic.hours[0].world_hash,
        "the folded WORLD is identical: a gap folds nothing in either stream"
    );
    assert_ne!(
        clock.hours[0].snapshot_hash, panic.hours[0].snapshot_hash,
        "the reason for an accounted absence is written into the hour's own published \
         provenance, so it is part of the hour the recording publishes -- not only of the \
         chain over it"
    );
    assert_ne!(
        chain_hash(&clock.hours),
        chain_hash(&panic.hours),
        "the reason for an accounted absence must change the hour's certified identity"
    );

    let gap_domain = clock.hours[0]
        .completeness
        .domains
        .get(&26)
        .expect("the gapped domain is accounted, not absent");
    assert_eq!(gap_domain.gap_records, 1);
    assert_eq!(gap_domain.perf_records, 0);
    assert_eq!(gap_domain.data_records, 0);
    assert_eq!(
        gap_reason_names(gap_domain.gap_reason_mask),
        ["ClockUnreadable"]
    );
    assert_eq!(gap_part_names(gap_domain.gap_part_mask), ["State"]);
}

#[test]
fn a_refused_frame_closes_its_hour_without_publishing_a_snapshot() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for (record_type, payload) in [
        (
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            country_identity_payload(&[(47, "GER")]),
        ),
        (COMPACT_RECORD_TYPE_DOMAIN_PERF, perf_payload()),
    ] {
        push_record(
            &mut stream,
            sequence,
            raw_for(HOUR_A),
            18,
            record_type,
            &payload,
        );
        sequence += 1;
    }
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal_payload(HOUR_B, 18),
    );

    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect("a stream that publishes one complete hour and refuses another certifies that hour");

    assert_eq!(
        run.hours
            .iter()
            .map(|hour| hour.hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_A],
        "a refused hour has no folded snapshot"
    );
    let refusal = run
        .refusals
        .get(&HOUR_B)
        .expect("the refused hour is accounted, not silently absent");
    assert_eq!(refusal.offenders.len(), 1);
    assert_eq!(refusal.offenders[0].domain_id, 18);
    assert_eq!(
        refusal.offenders[0].non_tolerated_flags,
        COMPACT_STATUS_SOURCE_UNAVAILABLE
    );
    assert_eq!(
        refusal.discarded_terminal_outcome_domains,
        vec![18],
        "the refusal terminated an hour that had already produced an outcome, and the report must say so"
    );
    assert!(
        refusal.frame_findings.is_empty(),
        "this fixture attributes its whole cause to a domain, so the report must \
         not invent a frame-level finding beside it"
    );
    assert_eq!(refusal.wire_version, 2);
    assert_eq!(
        refusal.offenders[0].status_flags,
        COMPACT_STATUS_SOURCE_UNAVAILABLE
    );
    assert_eq!(refusal.offenders[0].reason, "synthetic_offender=1");
}

fn stream_publishing_a_then_refusing_b(refusal: &[u8]) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for (record_type, payload) in [
        (
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            country_identity_payload(&[(47, "GER")]),
        ),
        (COMPACT_RECORD_TYPE_DOMAIN_PERF, perf_payload()),
    ] {
        push_record(
            &mut stream,
            sequence,
            raw_for(HOUR_A),
            18,
            record_type,
            &payload,
        );
        sequence += 1;
    }
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_REFUSED,
        refusal,
    );
    stream
}

#[test]
fn a_refusals_typed_diagnosis_is_decoded_and_reaches_the_report() {
    let stream = stream_publishing_a_then_refusing_b(&refusal_payload(HOUR_B, 18));
    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect("a version 2 refusal is a legal record and must not abandon the fold");

    assert_eq!(
        run.hours
            .iter()
            .map(|hour| hour.hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_A],
        "the hour before the refusal still certifies"
    );
    let refusal = run
        .refusals
        .get(&HOUR_B)
        .expect("the refused hour is accounted, not silently absent");
    assert_eq!(refusal.offenders.len(), 1);
    assert_eq!(refusal.offenders[0].domain_id, 18);
    assert_eq!(
        refusal.offenders[0].non_tolerated_flags,
        COMPACT_STATUS_SOURCE_UNAVAILABLE
    );
    assert_eq!(refusal.wire_version, 2);
    let detail = refusal.offenders[0]
        .detail
        .as_deref()
        .expect("a version 2 offender carries a typed diagnosis");
    assert!(
        detail.contains("predicate=synthetic_source_slot_is_bound#131073"),
        "the report must name WHICH check refused: {detail}"
    );
    assert!(
        detail.contains("authority=domain91"),
        "the report must name the authority, which is not this domain: {detail}"
    );
    assert!(
        detail.contains("region 41 of 96"),
        "the report must carry the locator path with its extent: {detail}"
    );
    assert!(
        detail.contains("reason_names=1/1"),
        "the report must state how much of the detail the reason alone conveys: {detail}"
    );
}

#[test]
fn the_refusals_typed_diagnosis_survives_the_json_boundary() {
    let stream = stream_publishing_a_then_refusing_b(&refusal_payload(HOUR_B, 18));
    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect("a refusal is a legal record on the wire");
    let rendered = refusals_json(&run.refusals);
    let hour = &rendered["hours"][0];
    assert_eq!(hour["wire_version"], json!(2u64), "{rendered}");
    let detail = &hour["offenders"][0]["detail"];
    assert!(
        detail
            .as_str()
            .expect("an offender carries a typed diagnosis")
            .contains("predicate=synthetic_source_slot_is_bound"),
        "{rendered}"
    );
}

#[test]
fn a_version_2_refusal_that_names_no_domain_still_says_why() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for hour in [HOUR_A, HOUR_B, HOUR_C] {
        push_record(
            &mut stream,
            sequence,
            raw_for(hour),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            &frame_only_refusal_payload(hour),
        );
        sequence += 1;
    }

    let dir = TmpDir::new("frame_only_refusal");
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, &stream).unwrap();
    let err = match run_input(&path, &one_domain_roster()) {
        Ok(_) => panic!("a stream that publishes no game hour must not certify"),
        Err(err) => err,
    };
    assert!(err.contains("publishes no game hour"), "{err}");
    assert!(err.contains("3 hour(s)"), "{err}");
    assert!(
        err.contains("frame-level finding predicate"),
        "the verdict must carry the cause a refusal that names no domain gave: {err}"
    );
    assert!(
        err.contains("frame_clock_is_readable"),
        "the verdict must name the predicate that refused: {err}"
    );
}

#[test]
fn a_refusal_version_this_build_does_not_decode_is_refused_by_name() {
    let stream = stream_publishing_a_then_refusing_b(&refusal_payload_at_version(HOUR_B, 18, 3));
    let dir = TmpDir::new("future_refusal_version");
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, &stream).unwrap();
    let err = match run_input(&path, &one_domain_roster()) {
        Ok(_) => panic!("a refusal version this build cannot decode must not certify silently"),
        Err(err) => err,
    };
    assert!(
        err.contains("stream validation at byte"),
        "the refusal is the record-semantics validator's, and must say where: {err}"
    );
    assert!(
        err.contains("frame refusal version 3"),
        "the refusal must name the version it could not read: {err}"
    );
    assert!(
        err.contains("not a version this build decodes"),
        "the refusal must say the reader is behind the producer, not that the record is corrupt: {err}"
    );
    assert!(
        err.contains("(2)"),
        "the refusal must name the version this build does decode: {err}"
    );
}

#[test]
fn a_stream_that_publishes_no_hour_certifies_nothing() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for hour in [HOUR_A, HOUR_B, HOUR_C] {
        push_record(
            &mut stream,
            sequence,
            raw_for(hour),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            &refusal_payload(hour, 18),
        );
        sequence += 1;
    }

    let dir = TmpDir::new("refusal_only");
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, &stream).unwrap();
    let err = match run_input(&path, &one_domain_roster()) {
        Ok(_) => panic!("a stream that publishes no game hour must not certify"),
        Err(err) => err,
    };
    assert!(err.contains("publishes no game hour"), "{err}");
    assert!(err.contains("3 hour(s)"), "{err}");
    assert!(
        err.contains("were attempted and refused"),
        "the verdict must name the refusals as the reason: {err}"
    );
}

#[test]
fn two_streams_that_refuse_different_hours_are_not_identical() {
    fn stream(refused_hour: i64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut sequence = 1u64;
        for (record_type, payload) in [
            (
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                country_identity_payload(&[(47, "GER")]),
            ),
            (COMPACT_RECORD_TYPE_DOMAIN_PERF, perf_payload()),
        ] {
            push_record(
                &mut out,
                sequence,
                raw_for(HOUR_A),
                18,
                record_type,
                &payload,
            );
            sequence += 1;
        }
        push_record(
            &mut out,
            sequence,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
        );
        sequence += 1;
        push_record(
            &mut out,
            sequence,
            raw_for(refused_hour),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            &refusal_payload(refused_hour, 18),
        );
        out
    }

    let dir = TmpDir::new("refusal_compare");
    let roster = one_domain_roster();
    let mut runs = Vec::new();
    for (name, refused_hour) in [("b.bin", HOUR_B), ("c.bin", HOUR_C)] {
        let path = dir.0.join(name);
        std::fs::write(&path, stream(refused_hour)).unwrap();
        runs.push(
            run_input(&path, &roster)
                .expect("published hour A certifies")
                .0,
        );
    }

    assert_eq!(runs[0].hours.len(), 1);
    assert_eq!(runs[1].hours.len(), 1);
    assert_eq!(
        runs[0].hours[0].snapshot_hash,
        runs[1].hours[0].snapshot_hash
    );
    assert_ne!(
        runs[0].refusals.keys().collect::<Vec<_>>(),
        runs[1].refusals.keys().collect::<Vec<_>>(),
        "the two streams refused different hours"
    );

    let divergence = refusal_divergence(&runs[0].refusals, &runs[1].refusals)
        .expect("differing refusals are a compare divergence");
    assert_eq!(divergence["kind"], "refused_hour_mismatch");
    assert_eq!(divergence["refused_only_in_a"], json!([HOUR_B]));
    assert_eq!(divergence["refused_only_in_b"], json!([HOUR_C]));
    assert!(
        refusal_divergence(&runs[0].refusals, &runs[0].refusals).is_none(),
        "identical refusals are not a divergence"
    );
}

#[test]
fn the_audit_accounts_every_record_type_the_contract_admits() {
    let mut admitted = Vec::new();
    for record_type in 0u16..=u16::MAX {
        for (domain_id, payload) in [
            (18u16, country_identity_payload(&[(47, "GER")])),
            (18, perf_payload()),
            (DOMAIN_FRAME, frame_end_payload_with(1, 1, 0)),
            (18, clock_unreadable_gap_payload(HOUR_A, 0)),
            (DOMAIN_FRAME, refusal_payload(HOUR_A, 18)),
        ] {
            if validate_compact_record_semantics(domain_id, record_type, 0, &payload).is_err() {
                continue;
            }
            admitted.push(record_type);
            let mut audit = CompletenessAudit::default();
            let outcome = audit.observe(raw_for(HOUR_A), domain_id, record_type, 0, 1, 0, &payload);
            if let Err(err) = outcome {
                assert!(
                    !err.contains("not accounted by the completeness audit"),
                    "record type {record_type} is admitted by the wire contract but the \
                     completeness audit does not account it: {err}"
                );
            }
        }
    }
    admitted.sort_unstable();
    admitted.dedup();
    assert_eq!(
        admitted,
        vec![
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_TYPE_FRAME_END,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>(),
        "the sweep must actually have reached every admitted record type"
    );
}

#[test]
fn an_hour_is_published_or_refused_but_never_both() {
    for refuse_first in [true, false] {
        let mut audit = CompletenessAudit::default();
        let steps: [(u16, u16, Vec<u8>); 2] = [
            (
                DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal_payload(HOUR_A, 18),
            ),
            (
                DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                frame_end_payload_with(0, 0, 0),
            ),
        ];
        let ordered: Vec<&(u16, u16, Vec<u8>)> = if refuse_first {
            vec![&steps[0], &steps[1]]
        } else {
            vec![&steps[1], &steps[0]]
        };
        audit
            .observe(
                raw_for(HOUR_A),
                ordered[0].0,
                ordered[0].1,
                0,
                1,
                0,
                &ordered[0].2,
            )
            .expect("the first close of the hour is legitimate");
        let err = audit
            .observe(
                raw_for(HOUR_A),
                ordered[1].0,
                ordered[1].1,
                0,
                2,
                0,
                &ordered[1].2,
            )
            .expect_err("the same hour cannot be closed twice with contradictory outcomes");
        assert!(
            err.contains("is both"),
            "refuse_first={refuse_first}: {err}"
        );
    }
}

#[test]
fn compare_reports_a_divergence_when_only_the_refusals_differ() {
    fn stream(refused_hour: i64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut sequence = 1u64;
        for (record_type, payload) in [
            (
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                country_identity_payload(&[(47, "GER")]),
            ),
            (COMPACT_RECORD_TYPE_DOMAIN_PERF, perf_payload()),
        ] {
            push_record(
                &mut out,
                sequence,
                raw_for(HOUR_A),
                18,
                record_type,
                &payload,
            );
            sequence += 1;
        }
        push_record(
            &mut out,
            sequence,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
        );
        sequence += 1;
        push_record(
            &mut out,
            sequence,
            raw_for(refused_hour),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            &refusal_payload(refused_hour, 18),
        );
        out
    }

    let dir = TmpDir::new("compare_refusals");
    let roster = one_domain_roster();
    let path_b = dir.0.join("refuses_b.bin");
    let path_c = dir.0.join("refuses_c.bin");
    std::fs::write(&path_b, stream(HOUR_B)).unwrap();
    std::fs::write(&path_c, stream(HOUR_C)).unwrap();
    let run_b = run_input(&path_b, &roster)
        .expect("published hour A certifies")
        .0;
    let run_c = run_input(&path_c, &roster)
        .expect("published hour A certifies")
        .0;

    let divergence = first_divergence(
        &run_b,
        &run_c,
        &path_b,
        &path_c,
        24,
        &roster,
        StreamIdentityMode::Legacy,
    )
    .expect("both runs certified, so the comparison itself must not error")
    .expect("two streams that refused different hours are not identical");
    assert_eq!(divergence["kind"], "refused_hour_mismatch");

    assert!(
        first_divergence(
            &run_b,
            &run_b,
            &path_b,
            &path_b,
            24,
            &roster,
            StreamIdentityMode::Legacy,
        )
        .expect("self-comparison must not error")
        .is_none(),
        "a run must be identical to itself"
    );
}

#[test]
fn a_refused_hours_records_do_not_reach_the_next_published_hour() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal_payload(HOUR_A, 18),
    );
    sequence += 1;
    for (record_type, payload) in [
        (
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            country_identity_payload(&[(48, "FRA")]),
        ),
        (COMPACT_RECORD_TYPE_DOMAIN_PERF, perf_payload()),
    ] {
        push_record(
            &mut stream,
            sequence,
            raw_for(HOUR_B),
            18,
            record_type,
            &payload,
        );
        sequence += 1;
    }
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );

    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect("neither the fold nor the audit may carry hour A's withheld record into hour B");

    assert_eq!(
        run.hours
            .iter()
            .map(|hour| hour.hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_B],
        "hour A was refused, so hour B is the only published hour"
    );
    assert_eq!(
        run.hours[0].domain_data_counts.get(&18).copied(),
        Some(1),
        "hour B publishes its own record only; hour A's withheld one must not land here"
    );
    assert!(
        run.refusals.contains_key(&HOUR_A),
        "the refused hour is accounted, not silently dropped"
    );
}

#[test]
fn a_bounded_refusal_report_still_states_the_exact_totals() {
    let mut refusals: BTreeMap<i64, HourRefusal> = BTreeMap::new();
    let total = REFUSAL_DETAIL_LIMIT + 17;
    for step in 0..total {
        refusals.insert(
            HOUR_A + step as i64,
            HourRefusal {
                wire_version: 2,
                offenders: vec![RefusalOffender {
                    domain_id: 18 + (step % 3) as u16,
                    status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    reason: "synthetic_offender=1".to_string(),
                    detail: Some(
                        "predicate=synthetic#1 authority=self synthetic_offender=1 \
                         units=1/1 complete reason=synthetic_offender=1 reason_names=1/1"
                            .to_string(),
                    ),
                }],
                frame_findings: vec![FrameFinding {
                    predicate: format!("synthetic_frame_finding_{step}"),
                    detail: format!(
                        "predicate=synthetic_frame_finding_{step}#1 authority=host \
                         reason=synthetic hour {step}"
                    ),
                }],
                discarded_terminal_outcome_domains: Vec::new(),
                discarded_data_records: 0,
            },
        );
    }

    let rendered = refusals_json(&refusals);
    assert_eq!(rendered["count"], total);
    assert_eq!(
        rendered["hour_range"],
        json!([HOUR_A, HOUR_A + total as i64 - 1])
    );
    assert_eq!(rendered["offending_domains"], json!([18, 19, 20]));
    assert_eq!(rendered["frame_finding_predicate_count"], total);
    assert_eq!(
        rendered["frame_finding_predicates"]
            .as_array()
            .expect("predicate list")
            .len(),
        FRAME_FINDING_PREDICATE_LIMIT,
        "the predicate list is bounded"
    );
    assert_eq!(
        rendered["frame_finding_predicate_limit"],
        FRAME_FINDING_PREDICATE_LIMIT
    );
    assert!(
        total > FRAME_FINDING_PREDICATE_LIMIT,
        "this fixture is only meaningful while it exceeds the bound"
    );
    let verdict =
        verify_published_coverage(Path::new("synthetic"), 0, &refusals, &one_domain_roster())
            .expect_err("a stream that publishes no game hour must not certify");
    assert!(
        verdict.contains(&format!("{total} frame-level finding predicate(s)")),
        "the verdict states the exact predicate count: {verdict}"
    );
    assert!(
        verdict.contains(&format!(
            "and {} more",
            total - FRAME_FINDING_PREDICATE_LIMIT
        )),
        "the verdict says how many predicates it elided: {verdict}"
    );
    assert!(
        !verdict.contains("synthetic_frame_finding_80"),
        "the verdict must not spill the whole union into one line: {verdict}"
    );

    assert_eq!(rendered["hours_detailed"], REFUSAL_DETAIL_LIMIT);
    assert_eq!(rendered["hours_detail_limit"], REFUSAL_DETAIL_LIMIT);
    assert_eq!(
        rendered["hours"].as_array().expect("detail array").len(),
        REFUSAL_DETAIL_LIMIT,
        "the detail is bounded"
    );
}

#[test]
fn a_data_payload_that_declares_failure_cannot_certify_behind_a_clean_perf() {
    // The shape that made publishing worse than the deadlock it replaced.
    fn stream(payload_status: u32) -> Vec<u8> {
        let mut data = country_identity_payload(&[(47, "GER")]);
        data[16..20].copy_from_slice(&payload_status.to_le_bytes());
        let mut out = Vec::new();
        push_record(
            &mut out,
            1,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &data,
        );
        push_record(
            &mut out,
            2,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(),
        );
        push_record(
            &mut out,
            3,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
        );
        out
    }

    let clean = stream(0);
    collect_certified_run(
        Cursor::new(clean.clone()),
        clean.len() as u64,
        &one_domain_roster(),
    )
    .expect("a clean payload under a clean PERF is an ordinary certifiable hour");

    let forged = stream(COMPACT_STATUS_SOURCE_UNAVAILABLE);
    let err = collect_certified_run(
        Cursor::new(forged.clone()),
        forged.len() as u64,
        &one_domain_roster(),
    )
    .err()
    .expect("a DATA payload declaring failure must not certify behind a clean PERF");
    assert!(
        err.contains("domain 18"),
        "the verdict must name the offending domain: {err}"
    );
    assert!(
        err.contains("DOMAIN_DATA"),
        "the verdict must name the payload's own status, not the PERF's: {err}"
    );
    assert!(err.contains("non-tolerated failure flags"), "{err}");
    assert!(
        err.contains("declares no H4FD degraded notice"),
        "the payload failure must refuse for being UNDECLARED, which is the fact \
         that distinguishes it from t26's accounted d86: {err}"
    );
}

#[test]
fn the_data_payload_status_is_part_of_the_hours_certified_identity() {
    fn stream(payload_status: u32) -> Vec<u8> {
        let mut data = country_identity_payload(&[(47, "GER")]);
        data[16..20].copy_from_slice(&payload_status.to_le_bytes());
        let mut out = Vec::new();
        push_record(
            &mut out,
            1,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &data,
        );
        push_record(
            &mut out,
            2,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(),
        );
        push_record(
            &mut out,
            3,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
        );
        out
    }

    let plain = fold_bytes(&stream(0));
    let delta = fold_bytes(&stream(COMPACT_STATUS_DELTA_DATA));
    assert_eq!(
        plain.hours[0].snapshot_hash, delta.hours[0].snapshot_hash,
        "the tolerated status changes no folded row"
    );
    assert_eq!(
        plain.hours[0]
            .completeness
            .domains
            .get(&18)
            .expect("domain 18 is accounted")
            .data_payload_status_flags_or,
        0
    );
    assert_ne!(
        delta.hours[0]
            .completeness
            .domains
            .get(&18)
            .expect("domain 18 is accounted")
            .data_payload_status_flags_or,
        0,
        "the payload's own status must be recorded, not discarded"
    );
    assert_ne!(
        chain_hash(&plain.hours),
        chain_hash(&delta.hours),
        "the DATA payload's status must change the hour's certified identity"
    );

    fn stream_with_perf_status(perf_status: u32, payload_status: u32) -> Vec<u8> {
        let mut data = country_identity_payload(&[(47, "GER")]);
        data[16..20].copy_from_slice(&payload_status.to_le_bytes());
        let mut out = Vec::new();
        push_record(
            &mut out,
            1,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &data,
        );
        push_record(
            &mut out,
            2,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(perf_status),
        );
        push_record(
            &mut out,
            3,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
        );
        out
    }

    let perf_only = fold_bytes(&stream_with_perf_status(COMPACT_STATUS_DELTA_DATA, 0));
    let both = fold_bytes(&stream_with_perf_status(
        COMPACT_STATUS_DELTA_DATA,
        COMPACT_STATUS_DELTA_DATA,
    ));
    let perf_only_domain = perf_only.hours[0]
        .completeness
        .domains
        .get(&18)
        .expect("domain 18 is accounted");
    let both_domain = both.hours[0]
        .completeness
        .domains
        .get(&18)
        .expect("domain 18 is accounted");
    assert_eq!(
        perf_only_domain.normalized_status_flags_or, both_domain.normalized_status_flags_or,
        "the merged per-domain status is identical, which is the whole point"
    );
    assert_ne!(
        perf_only_domain.data_payload_status_flags_or,
        both_domain.data_payload_status_flags_or
    );
    assert_ne!(
        chain_hash(&perf_only.hours),
        chain_hash(&both.hours),
        "which record carried the status must change the certified identity"
    );
}

fn degraded_hour_stream(
    perf_domains: &[(u16, u32)],
    declared: Option<&[(u16, u32, &str)]>,
) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    sequence += 1;
    for (domain_id, status_flags) in perf_domains {
        push_record(
            &mut stream,
            sequence,
            raw_for(HOUR_A),
            *domain_id,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(*status_flags),
        );
        sequence += 1;
    }
    if let Some(entries) = declared {
        let notice = compact_contract::frame_degraded::FrameDegradedRecordV2 {
            degraded_hour: raw_for(HOUR_A),
            domains: entries
                .iter()
                .map(|(domain_id, status_flags, reason)| {
                    compact_contract::frame_degraded::FrameDegradedDomain {
                        domain_id: *domain_id,
                        status_flags: *status_flags,
                        non_tolerated_flags:
                            compact_status_failure_flags_after_expected_partial_tolerance(
                                *domain_id,
                                *status_flags,
                            ),
                        predicate: compact_contract::diagnostic_channel::Predicate::undeclared(),
                        authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
                        reason: (*reason).to_string(),
                    }
                })
                .collect(),
        };
        let payload = notice.encode().expect("test notice must be encodable");
        push_record(
            &mut stream,
            sequence,
            raw_for(HOUR_A),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_DEGRADED,
            &payload,
        );
        sequence += 1;
    }
    let terminal_outcomes = perf_domains.len() as u32;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(terminal_outcomes, terminal_outcomes, 0),
    );
    stream
}

#[test]
fn a_declared_degradation_the_status_does_not_support_fails_certification() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    sequence += 1;
    for domain_id in [18u16, 30] {
        push_record(
            &mut stream,
            sequence,
            raw_for(HOUR_A),
            domain_id,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0),
        );
        sequence += 1;
    }
    let notice = compact_contract::frame_degraded::FrameDegradedRecordV2 {
        degraded_hour: raw_for(HOUR_A),
        domains: vec![compact_contract::frame_degraded::FrameDegradedDomain {
            domain_id: 30,
            status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            predicate: compact_contract::diagnostic_channel::Predicate::undeclared(),
            authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
            reason: "d30_root_unreadable=1".to_string(),
        }],
    };
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_DEGRADED,
        &notice.encode().expect("a self-consistent notice encodes"),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(2, 2, 0),
    );
    let err = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .err()
        .expect("a fabricated degradation must not certify");
    assert!(
        err.contains("declares domain 30 degraded")
            && err.contains("no non-tolerated failure flags"),
        "{err}"
    );
}

#[test]
fn a_frame_declaring_its_degraded_domains_certifies_and_the_hour_records_them_unknown() {
    let stream = degraded_hour_stream(
        &[
            (18, 0),
            (29, COMPACT_STATUS_PARTIAL_DATA),
            (30, COMPACT_STATUS_SOURCE_UNAVAILABLE),
        ],
        Some(&[
            (
                29,
                COMPACT_STATUS_PARTIAL_DATA,
                "d29_invalid_air_wing_rows=3",
            ),
            (
                30,
                COMPACT_STATUS_SOURCE_UNAVAILABLE,
                "d30_root_unreadable=1",
            ),
        ]),
    );
    let run = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .expect("a frame that declares its degraded domains must certify");

    let hour = run
        .hours
        .iter()
        .find(|hour| hour.hour_index == HOUR_A)
        .expect("the degraded hour must be published, not skipped");

    assert_eq!(
        hour.domain_data_counts.get(&18),
        Some(&1),
        "the domains that read fine must publish; that is the entire point"
    );

    assert_eq!(
        hour.completeness.declared_degraded_domains,
        [
            (
                29u16,
                (COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_PARTIAL_DATA)
            ),
            (
                30u16,
                (
                    COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    COMPACT_STATUS_SOURCE_UNAVAILABLE
                )
            )
        ]
        .into_iter()
        .collect::<BTreeMap<u16, (u32, u32)>>(),
        "a degraded domain must be certified as unknown, never as complete, and the notice's \
         RAW status word must be the one the PERF record published"
    );
    assert!(
        !hour
            .completeness
            .declared_degraded_domains
            .contains_key(&18),
        "a clean domain must never appear in the degraded declaration"
    );
}

#[test]
fn a_notice_framed_for_another_hour_fails_certification() {
    let mut stream = degraded_hour_stream(
        &[(18, 0), (29, COMPACT_STATUS_PARTIAL_DATA)],
        Some(&[(
            29,
            COMPACT_STATUS_PARTIAL_DATA,
            "d29_invalid_air_wing_rows=3",
        )]),
    );
    let magic = compact_contract::frame_degraded::FRAME_DEGRADED_MAGIC;
    let at = stream
        .windows(4)
        .position(|window| window == magic)
        .expect("the stream carries an H4FD payload");
    stream[at + 8..at + 12].copy_from_slice(&raw_for(HOUR_B).to_le_bytes());
    let err = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .err()
        .expect("a notice framed for another hour must not certify");
    assert!(
        err.contains("FRAME_DEGRADED") && err.contains("at byte"),
        "the boundary tracker must be the layer that catches this: {err}"
    );
}

#[test]
fn a_tolerated_partial_certifies_and_is_never_declared_degraded() {
    let stream = degraded_hour_stream(&[(18, 0), (26, COMPACT_STATUS_PARTIAL_DATA)], None);
    let run = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .expect("a tolerated partial certifies without any H4FD notice");
    let hour = run
        .hours
        .iter()
        .find(|hour| hour.hour_index == HOUR_A)
        .expect("the hour must publish");
    assert!(
        hour.completeness.declared_degraded_domains.is_empty(),
        "an expected partial must never be labelled a degradation"
    );

    let report = completeness_json(&hour.completeness);
    assert_eq!(
        report["domains"]["26"]["state"],
        serde_json::json!("expected_partial"),
        "{report:#}"
    );
    assert_eq!(
        report["domains"]["26"]["complete"],
        serde_json::json!(false),
        "an expected partial is not complete: {report:#}"
    );
    assert_eq!(
        report["domains"]["18"]["state"],
        serde_json::json!("complete"),
        "{report:#}"
    );
    assert_eq!(
        report["domains"]["18"]["complete"],
        serde_json::json!(true),
        "{report:#}"
    );

    let payload_side_partial = DomainCompleteness {
        data_records: 1,
        perf_records: 1,
        data_payload_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
        normalized_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
        ..DomainCompleteness::default()
    };
    assert_eq!(
        compact_status_failure_flags_after_expected_partial_tolerance(
            26,
            COMPACT_STATUS_PARTIAL_DATA,
        ),
        0,
        "domain 26 must tolerate PARTIAL_DATA outright, or this is no longer an \
         expected-partial fixture"
    );
    assert_eq!(
        payload_side_partial.state(26),
        "expected_partial",
        "a tolerated partial reaching the hour only through the DATA payload is still an \
         expected partial, and an expected partial is not complete"
    );
    assert!(!payload_side_partial.is_complete(26));
    assert_eq!(
        DomainCompleteness {
            data_records: 1,
            perf_records: 1,
            data_payload_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
            normalized_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
            non_tolerated_data_payload_status_flags:
                compact_status_failure_flags_after_expected_partial_tolerance(
                    18,
                    COMPACT_STATUS_PARTIAL_DATA,
                ),
            ..DomainCompleteness::default()
        }
        .state(18),
        "degraded_unknown"
    );

    assert!(
        compact_contract::frame_degraded::FrameDegradedDomain::from_status(
            26,
            COMPACT_STATUS_PARTIAL_DATA,
            fixture_diagnosis("mislabelled"),
        )
        .is_none(),
        "a tolerated partial must not produce a degraded entry"
    );
}

#[test]
fn a_d86_producer_declared_partial_hour_certifies_as_expected_partial() {
    assert_eq!(
        compact_status_failure_flags_after_expected_partial_tolerance(
            86,
            COMPACT_STATUS_PARTIAL_DATA,
        ),
        0,
        "domain 86 must tolerate PARTIAL_DATA outright"
    );
    let partial_hour = DomainCompleteness {
        data_records: 1,
        perf_records: 1,
        data_payload_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
        normalized_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
        ..DomainCompleteness::default()
    };
    assert_eq!(partial_hour.state(86), "expected_partial");
    assert!(
        !partial_hour.is_complete(86),
        "a partial delivery is not a complete one"
    );

    assert_eq!(
        DomainCompleteness {
            data_records: 1,
            perf_records: 1,
            ..DomainCompleteness::default()
        }
        .state(86),
        "complete"
    );

    assert!(
        compact_contract::frame_degraded::FrameDegradedDomain::from_status(
            86,
            COMPACT_STATUS_PARTIAL_DATA,
            fixture_diagnosis("mislabelled"),
        )
        .is_none(),
        "a tolerated d86 partial must not produce a degraded entry"
    );
}

#[test]
fn an_undeclared_non_tolerated_failure_still_fails_certification() {
    let stream = degraded_hour_stream(&[(18, 0), (29, COMPACT_STATUS_PARTIAL_DATA)], None);
    let err = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .err()
        .expect("an undeclared non-tolerated failure must not certify");
    assert!(err.contains("domain 29"), "{err}");
    assert!(err.contains("non-tolerated failure flags"), "{err}");
    assert!(
        err.contains("declares no H4FD degraded notice"),
        "the verdict must say the declaration was missing, not merely that a bit was set: {err}"
    );
}

fn sunk_ship_payload(
    status_flags: u32,
    producer_loss: Option<sunk_ship_history_live_lane::SunkShipProducerLoss>,
) -> Vec<u8> {
    sunk_ship_payload_for(HOUR_A, status_flags, producer_loss)
}

fn sunk_ship_payload_for(
    hour_index: i64,
    status_flags: u32,
    producer_loss: Option<sunk_ship_history_live_lane::SunkShipProducerLoss>,
) -> Vec<u8> {
    use sunk_ship_history_live_lane as d86;
    if status_flags != 0 {
        return d86::encode_unavailable(hour_index as u64, 7, status_flags)
            .expect("an unavailable d86 payload encodes");
    }
    let snapshot_status = if producer_loss.is_some() {
        COMPACT_STATUS_PARTIAL_DATA
    } else {
        0
    };
    d86::encode_payload(&d86::SunkShipFrame {
        elapsed_wall_ms: 7,
        frame_hour: hour_index as u64,
        status_flags: snapshot_status,
        authority: d86::AuthorityState::StaticExact20Receipt,
        snapshot: Some(d86::SunkShipSnapshot {
            source_sequence: 2,
            producer_complete: producer_loss.is_none(),
            records_with_unavailable_text: 0,
            producer_hour_raw: None,
            producer_loss,
            events: vec![d86::SunkShipEvent {
                event_sequence: 2,
                combat_join_present: true,
                combat_id: 4,
                combat_generation: 2,
                victim_country_id_raw: 3,
                attacker_country_id_raw: 9,
                ui_source_country_id_raw: 5,
                sink_date_raw: DATE_EPOCH_CONST + 16_959_300,
                victim_member_battle_slot_index: 7,
                flags: d86::EVENT_FLAG_KILLED,
                victim_ship_reference_raw: [11, 12],
                damage_source_reference_raw: [13, 14],
                task_force_id_raw: 15,
                terminal_context_raw: [16, 17],
                cause_record_flag: false,
                ship_name: "HMS Exact".into(),
                ship_class_name: "Destroyer".into(),
                attribution_source_name: "KMS Exact".into(),
                attribution_source_class_name: "Cruiser".into(),
            }],
        }),
    })
    .expect("a d86 snapshot payload encodes")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum D86HourClose {
    Published,
    Refused,
}

fn d86_hour_stream(payload: &[u8], perf_status: u32, declared: Option<(u32, &str)>) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    append_d86_hour(
        &mut stream,
        &mut sequence,
        HOUR_A,
        payload,
        perf_status,
        declared,
        D86HourClose::Published,
    );
    stream
}

#[allow(clippy::too_many_arguments)]
fn append_d86_hour(
    stream: &mut Vec<u8>,
    sequence: &mut u64,
    hour_index: i64,
    payload: &[u8],
    perf_status: u32,
    declared: Option<(u32, &str)>,
    close: D86HourClose,
) {
    push_record(
        stream,
        *sequence,
        raw_for(hour_index),
        86,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        payload,
    );
    *sequence += 1;
    push_record(
        stream,
        *sequence,
        raw_for(hour_index),
        86,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload_with_status(perf_status),
    );
    *sequence += 1;
    if let Some((status_flags, reason)) = declared {
        let notice = compact_contract::frame_degraded::FrameDegradedRecordV2 {
            degraded_hour: raw_for(hour_index),
            domains: vec![compact_contract::frame_degraded::FrameDegradedDomain {
                domain_id: 86,
                status_flags,
                non_tolerated_flags: compact_status_failure_flags_after_expected_partial_tolerance(
                    86,
                    status_flags,
                ),
                predicate: compact_contract::diagnostic_channel::Predicate::undeclared(),
                authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
                reason: reason.to_string(),
            }],
        };
        push_record(
            stream,
            *sequence,
            raw_for(hour_index),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_DEGRADED,
            &notice.encode().expect("test notice must be encodable"),
        );
        *sequence += 1;
    }
    match close {
        D86HourClose::Published => push_record(
            stream,
            *sequence,
            raw_for(hour_index),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
        ),
        D86HourClose::Refused => push_record(
            stream,
            *sequence,
            raw_for(hour_index),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            &refusal_payload(hour_index, 86),
        ),
    }
    *sequence += 1;
}

fn d86_roster() -> ExpectedRoster {
    ExpectedRoster::test(&[(86, true)])
}

/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`):
#[test]
fn a_declared_d86_payload_failure_certifies_as_an_accounted_unknown() {
    let stream = d86_hour_stream(
        &sunk_ship_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE, None),
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        Some((
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            "domain 86: read_completion=2,answer_extent=1,\
             d86_refusal_code_4_dropped_unbound=1",
        )),
    );
    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &d86_roster(),
    )
    .expect("an accounted d86 failure must certify as a declared unknown");
    let hour = run
        .hours
        .iter()
        .find(|hour| hour.hour_index == HOUR_A)
        .expect("the hour must publish");
    assert_eq!(
        hour.completeness.declared_degraded_domains.get(&86),
        Some(&(
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            COMPACT_STATUS_SOURCE_UNAVAILABLE
        )),
        "the certified hour must record d86 as unknown with its exact bits"
    );
    assert_eq!(
        hour.completeness
            .domains
            .get(&86)
            .map(|domain| domain.data_payload_status_flags_or),
        Some(COMPACT_STATUS_SOURCE_UNAVAILABLE),
        "the payload's own status must remain in the certified evidence"
    );

    let rendered = completeness_json(&hour.completeness);
    assert_eq!(
        rendered["declared_degraded_domains"]["86"]["non_tolerated_flags"],
        json!(COMPACT_STATUS_SOURCE_UNAVAILABLE),
        "the report must name the declared-unknown domain: {rendered}"
    );
    assert_eq!(
        rendered["declared_degraded_domains"]["86"]["non_tolerated_flag_names"],
        json!(["source_unavailable"]),
        "the report must spell the bit, not only its value: {rendered}"
    );
}

/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): deleting the
#[test]
fn an_undeclared_d86_payload_failure_still_refuses() {
    let stream = d86_hour_stream(
        &sunk_ship_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE, None),
        0,
        None,
    );
    let err = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &d86_roster(),
    )
    .err()
    .expect("an unnamed d86 payload failure must never certify");
    assert!(err.contains("domain 86"), "{err}");
    assert!(err.contains("non-tolerated failure flags"), "{err}");
    assert!(
        err.contains("declares no H4FD degraded notice"),
        "the verdict must name the missing declaration, not merely the bit: {err}"
    );
}

#[test]
fn a_d86_declaration_with_the_wrong_bits_refuses() {
    let stream = d86_hour_stream(
        &sunk_ship_payload(
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
            None,
        ),
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        Some((
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            "domain 86: read_completion=2",
        )),
    );
    let err = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &d86_roster(),
    )
    .err()
    .expect("a declaration that understates the payload's bits must not certify");
    assert!(
        err.contains("domain 86 DOMAIN_DATA payload status"),
        "{err}"
    );
    assert!(
        err.contains("does not declare") && err.contains("0x00000300"),
        "the verdict must show both words: {err}"
    );
}

/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): restoring the
#[test]
fn a_notice_may_not_be_satisfied_by_the_union_of_two_disagreeing_statuses() {
    let stream = d86_hour_stream(
        &sunk_ship_payload(COMPACT_STATUS_WORKER_PANIC, None),
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        Some((
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_WORKER_PANIC,
            "domain 86: read_completion=2",
        )),
    );
    let err = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &d86_roster(),
    )
    .err()
    .expect("a notice declaring the union of two disagreeing words must not certify");
    assert!(err.contains("domain 86 PERF status"), "{err}");
    assert!(
        err.contains("0x00000100") && err.contains("0x00000140"),
        "the verdict must show the PERF residual and what the notice claims: {err}"
    );
}

/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): deleting the
#[test]
fn a_refused_frames_declaration_does_not_certify_the_next_hours_failure() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    append_d86_hour(
        &mut stream,
        &mut sequence,
        HOUR_A,
        &sunk_ship_payload_for(HOUR_A, COMPACT_STATUS_SOURCE_UNAVAILABLE, None),
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        Some((
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            "domain 86: read_completion=2",
        )),
        D86HourClose::Refused,
    );
    append_d86_hour(
        &mut stream,
        &mut sequence,
        HOUR_B,
        &sunk_ship_payload_for(HOUR_B, COMPACT_STATUS_SOURCE_UNAVAILABLE, None),
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        None,
        D86HourClose::Published,
    );

    let err = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &d86_roster(),
    )
    .err()
    .expect("hour A's declaration must not certify hour B's undeclared failure");
    assert!(
        err.contains("H4FD degraded notice") && err.contains("FRAME_REFUSED"),
        "the verdict must name the contradiction between the two frame records: {err}"
    );
}

/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): setting
#[test]
fn a_cd_style_partial_batch_certifies_without_a_degraded_notice() {
    let loss = sunk_ship_history_live_lane::SunkShipProducerLoss {
        dropped_unbound: 9,
        ..Default::default()
    };
    assert!(loss.names_a_cause(), "the fixture must name a real cause");
    let payload = sunk_ship_payload(0, Some(loss));
    let decoded =
        sunk_ship_history_live_lane::decode_payload(&payload).expect("the partial batch decodes");
    let snapshot = decoded.snapshot.expect("the batch carries its rows");
    assert!(
        !snapshot.producer_complete && snapshot.producer_loss == Some(loss),
        "the fixture must be a NAMED partial batch, not a whole one"
    );
    assert_eq!(
        snapshot.events.len(),
        1,
        "a partial batch still ships the rows that survived"
    );

    let stream = d86_hour_stream(&payload, 0, None);
    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &d86_roster(),
    )
    .expect("a named partial batch must certify; refusing it is what stops t27");
    let hour = run
        .hours
        .iter()
        .find(|hour| hour.hour_index == HOUR_A)
        .expect("the hour must publish");
    assert!(
        hour.completeness.declared_degraded_domains.is_empty(),
        "a named partial batch is not a degradation and must not be declared one"
    );
    assert_eq!(
        hour.domain_data_counts.get(&86),
        Some(&1),
        "the batch's rows must reach the certified hour"
    );
}

#[test]
fn d86_failed_reads_certify_through_the_declaration_and_never_through_the_tolerance() {
    let tolerance = compact_contract::expected_partial_tolerance(86)
        .expect("d86's producer-declared partial is an expected partial");
    assert_eq!(
        tolerance.tolerated_flags, COMPACT_STATUS_PARTIAL_DATA,
        "exactly one bit is tolerated, and it is the producer's own partial declaration"
    );
    for (label, failure) in [
        ("source_unavailable", COMPACT_STATUS_SOURCE_UNAVAILABLE),
        ("worker_panic", COMPACT_STATUS_WORKER_PANIC),
        ("no_clock", COMPACT_STATUS_NO_CLOCK),
    ] {
        assert_eq!(
            compact_status_failure_flags_after_expected_partial_tolerance(86, failure),
            failure,
            "{label} must stay a non-tolerated failure for d86"
        );
        assert!(
            compact_contract::frame_degraded::FrameDegradedDomain::from_status(
                86,
                failure,
                fixture_diagnosis("domain 86: read_completion=2"),
            )
            .is_some(),
            "{label}: the accounting path for a d86 FAILURE is the H4FD declaration"
        );
    }
    assert_eq!(
        compact_status_failure_flags_after_expected_partial_tolerance(
            86,
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        ),
        COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        "a partial declaration does not tolerate the failure sitting beside it"
    );
}

const SHIPS_DOMAIN: u16 = 29;
// The d29 wire shape comes from `wire_format::d29`, which is version-pinned to
// `world_fold.rs::SHIP_PAYLOAD_VERSION`. Hand-copied literals here silently
// fell a version behind lane SR's v2 bump and every ships record these tests
// built was excluded by the decoder.
use wire_format::d29 as ships_wire;

type Ship = (u32, u32);

fn ships_payload(live: &[Ship], deleted: &[u32], status_flags: u32) -> Vec<u8> {
    let row_count = live.len() + deleted.len();
    let mut payload =
        Vec::with_capacity(ships_wire::HEADER_LEN + row_count * (ships_wire::FIXED_ROW_LEN + 16));
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&(row_count as u32).to_le_bytes());
    payload.extend_from_slice(&(row_count as u32).to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(live.len() as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(deleted.len() as u32).to_le_bytes());
    payload.extend_from_slice(&ships_wire::FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&ships_wire::PAYLOAD_VERSION.to_le_bytes());
    assert_eq!(
        payload.len(),
        ships_wire::HEADER_LEN,
        "ships header is {} bytes",
        ships_wire::HEADER_LEN
    );

    let mut push_row = |instance_id: u32, strength_raw: u32, definition: &str, flags: u32| {
        payload.extend_from_slice(&instance_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&strength_raw.to_le_bytes());
        payload.extend_from_slice(&50_000u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        // `task_force_id_space`: zero is the only legal value while
        // `FLAG_TASK_FORCE` is clear, which it is on every row here.
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&(definition.len() as u16).to_le_bytes());
        payload.extend_from_slice(definition.as_bytes());
    };
    for (instance_id, strength_raw) in live {
        push_row(
            *instance_id,
            *strength_raw,
            "destroyer_1",
            ships_wire::FLAG_OWNER | ships_wire::FLAG_DEFINITION,
        );
    }
    for instance_id in deleted {
        push_row(*instance_id, 0, "", ships_wire::FLAG_DELETED);
    }
    payload
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HourCShape {
    RetainedPinDelta,
    BlanketResetKeyframe,
    DivergentDelta,
}

fn ships_delta_chain_stream(foreign_gap: bool, shape: HourCShape) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let mut push = |stream: &mut Vec<u8>, hour: i64, domain: u16, rtype: u16, payload: &[u8]| {
        push_record(stream, seq, raw_for(hour), domain, rtype, payload);
        seq += 1;
    };

    push(
        &mut stream,
        HOUR_A,
        SHIPS_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &ships_payload(&[(1, 500)], &[], 0),
    );
    push(
        &mut stream,
        HOUR_A,
        SHIPS_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    push(
        &mut stream,
        HOUR_A,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );

    push(
        &mut stream,
        HOUR_B,
        SHIPS_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &ships_payload(&[(2, 700)], &[], COMPACT_STATUS_DELTA_DATA),
    );
    push(
        &mut stream,
        HOUR_B,
        SHIPS_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload_with_status(COMPACT_STATUS_DELTA_DATA),
    );
    if foreign_gap {
        push(
            &mut stream,
            HOUR_B,
            64,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            &clock_unreadable_gap_payload(HOUR_B, 0),
        );
    }
    let hour_b_outcomes = if foreign_gap { 2 } else { 1 };
    push(
        &mut stream,
        HOUR_B,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(hour_b_outcomes, hour_b_outcomes, 0),
    );

    let (payload, status) = match shape {
        HourCShape::RetainedPinDelta => (
            ships_payload(&[(1, 900)], &[], COMPACT_STATUS_DELTA_DATA),
            COMPACT_STATUS_DELTA_DATA,
        ),
        HourCShape::BlanketResetKeyframe => (ships_payload(&[(1, 900), (2, 700)], &[], 0), 0),
        HourCShape::DivergentDelta => (
            ships_payload(&[(1, 111)], &[], COMPACT_STATUS_DELTA_DATA),
            COMPACT_STATUS_DELTA_DATA,
        ),
    };
    push(
        &mut stream,
        HOUR_C,
        SHIPS_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &payload,
    );
    push(
        &mut stream,
        HOUR_C,
        SHIPS_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload_with_status(status),
    );
    push(
        &mut stream,
        HOUR_C,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );

    stream
}

#[test]
fn a_retained_delta_baseline_across_a_foreign_gap_folds_identically_to_the_keyframe() {
    let retained = fold_bytes(&ships_delta_chain_stream(
        true,
        HourCShape::RetainedPinDelta,
    ));
    let blanket = fold_bytes(&ships_delta_chain_stream(
        true,
        HourCShape::BlanketResetKeyframe,
    ));

    assert_eq!(
        retained.hours.len(),
        blanket.hours.len(),
        "both producers publish the same three hours"
    );
    for (a, b) in retained.hours.iter().zip(blanket.hours.iter()) {
        assert_eq!(
            a.hour_index, b.hour_index,
            "the two streams cover the same hours in the same order"
        );
        assert_eq!(
            a.snapshot_hash, b.snapshot_hash,
            "hour {} folds to a different world when domain 29's pin is retained across \
             domain 64's gap: delta {} vs keyframe {}",
            a.hour_index, a.snapshot_hash, b.snapshot_hash
        );
    }
}

#[test]
fn the_delta_and_keyframe_shapes_agree_with_and_without_the_foreign_gap() {
    for foreign_gap in [true, false] {
        let delta = fold_bytes(&ships_delta_chain_stream(
            foreign_gap,
            HourCShape::RetainedPinDelta,
        ));
        let keyframe = fold_bytes(&ships_delta_chain_stream(
            foreign_gap,
            HourCShape::BlanketResetKeyframe,
        ));
        assert_eq!(
            delta
                .hours
                .last()
                .expect("hour C is published")
                .snapshot_hash,
            keyframe
                .hours
                .last()
                .expect("hour C is published")
                .snapshot_hash,
            "hour C must fold identically whether or not domain 64 gapped (foreign_gap = \
             {foreign_gap})"
        );
    }
    let with_gap = fold_bytes(&ships_delta_chain_stream(
        true,
        HourCShape::RetainedPinDelta,
    ));
    let without_gap = fold_bytes(&ships_delta_chain_stream(
        false,
        HourCShape::RetainedPinDelta,
    ));
    for (a, b) in with_gap.hours.iter().zip(without_gap.hours.iter()) {
        assert_eq!(
            a.world_hash, b.world_hash,
            "hour {} changed world merely because domain 64 gapped",
            a.hour_index
        );
    }
    assert_ne!(
        with_gap.hours[1].snapshot_hash, without_gap.hours[1].snapshot_hash,
        "hour B is the hour domain 64 gapped in; the gap is an accounted absence the hour has \
         to state, so the two streams cannot publish the same hour B"
    );
    for hour in [0usize, 2] {
        assert_eq!(
            with_gap.hours[hour].snapshot_hash, without_gap.hours[hour].snapshot_hash,
            "hour {} does not carry domain 64's gap and must be published byte-identically in \
             both streams; a gap that outlived its own hour would show up here",
            with_gap.hours[hour].hour_index
        );
    }
}

#[test]
fn the_ships_fold_equivalence_gate_can_tell_two_worlds_apart() {
    let world = fold_bytes(&ships_delta_chain_stream(
        true,
        HourCShape::RetainedPinDelta,
    ));
    let other = fold_bytes(&ships_delta_chain_stream(true, HourCShape::DivergentDelta));

    assert_eq!(
        world.hours[0].snapshot_hash, other.hours[0].snapshot_hash,
        "the two streams are identical up to hour C, so hour A must still match"
    );
    assert_eq!(
        world.hours[1].snapshot_hash, other.hours[1].snapshot_hash,
        "hour B is identical in both streams"
    );
    assert_ne!(
        world.hours[2].snapshot_hash, other.hours[2].snapshot_hash,
        "hour C carries a different world in the two streams, so the gate above is comparing \
         something that can differ"
    );
}

#[test]
fn the_hour_c_delta_merges_onto_the_retained_baseline() {
    let retained = fold_bytes(&ships_delta_chain_stream(
        true,
        HourCShape::RetainedPinDelta,
    ));

    let mut tombstoned = Vec::new();
    let mut seq = 1u64;
    let mut push = |stream: &mut Vec<u8>, hour: i64, domain: u16, rtype: u16, payload: &[u8]| {
        push_record(stream, seq, raw_for(hour), domain, rtype, payload);
        seq += 1;
    };
    for (hour, live, deleted, status) in [
        (HOUR_A, vec![(1u32, 500u32)], vec![], 0u32),
        (HOUR_B, vec![(2, 700)], vec![], COMPACT_STATUS_DELTA_DATA),
        (
            HOUR_C,
            vec![(1, 900)],
            vec![2u32],
            COMPACT_STATUS_DELTA_DATA,
        ),
    ] {
        push(
            &mut tombstoned,
            hour,
            SHIPS_DOMAIN,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &ships_payload(&live, &deleted, status),
        );
        push(
            &mut tombstoned,
            hour,
            SHIPS_DOMAIN,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(status),
        );
        let outcomes = if hour == HOUR_B {
            push(
                &mut tombstoned,
                hour,
                64,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                &clock_unreadable_gap_payload(HOUR_B, 0),
            );
            2
        } else {
            1
        };
        push(
            &mut tombstoned,
            hour,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(outcomes, outcomes, 0),
        );
    }
    let without_ship2 = fold_bytes(&tombstoned);

    assert_eq!(
        retained.hours[1].snapshot_hash, without_ship2.hours[1].snapshot_hash,
        "the two streams are identical through hour B"
    );
    assert_ne!(
        retained.hours[2].snapshot_hash, without_ship2.hours[2].snapshot_hash,
        "hour C's delta must have merged onto the retained baseline and KEPT ship 2; a world \
         that tombstoned it has to hash differently, or the equivalence gate is comparing two \
         worlds that both lost the same row"
    );
}

#[test]
fn the_middle_hour_really_carries_the_foreign_domains_gap() {
    let with_gap = fold_bytes(&ships_delta_chain_stream(
        true,
        HourCShape::RetainedPinDelta,
    ));
    let without_gap = fold_bytes(&ships_delta_chain_stream(
        false,
        HourCShape::RetainedPinDelta,
    ));
    assert!(
        with_gap.hours[1].completeness.domains.contains_key(&64),
        "domain 64 must be accounted in hour B; its gap is the rejected bracket this gate \
         exists for"
    );
    assert!(
        !without_gap.hours[1].completeness.domains.contains_key(&64),
        "the control stream must genuinely lack that gap"
    );
    assert!(
        with_gap.hours[1]
            .completeness
            .domains
            .contains_key(&SHIPS_DOMAIN),
        "domain 29 must still be accounted in the same hour"
    );
}

const TECH_DOMAIN: u16 = 13;
const TECH_HEADER_LEN: usize = 32;
const TECH_PROGRESS_SCALE_RAW: u32 = 1_000;
const TECH_LIVE_PROGRESS_SCALE_RAW: u32 = 100_000;
const TECH_COUNTRY_FLAG_COMPLETED: u32 = 1;
const TECH_COUNTRY_FLAG_SLOTS: u32 = 1 << 1;

type TechCountry = (u32, u32, u32);

fn tech_payload(live: &[TechCountry], status_flags: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(TECH_HEADER_LEN + live.len() * 16);
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&(live.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(live.len() as u32).to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&TECH_PROGRESS_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&TECH_LIVE_PROGRESS_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(payload.len(), TECH_HEADER_LEN, "tech header is 32 bytes");
    for (tag, completed, slots) in live {
        payload.extend_from_slice(&tag.to_le_bytes());
        payload.extend_from_slice(
            &(TECH_COUNTRY_FLAG_COMPLETED | TECH_COUNTRY_FLAG_SLOTS).to_le_bytes(),
        );
        payload.extend_from_slice(&completed.to_le_bytes());
        payload.extend_from_slice(&slots.to_le_bytes());
    }
    payload
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TechHourCShape {
    Patch,
    WholePayload,
    PatchReassertingTheUnmeasuredCountry,
    DivergentPatch,
}

fn tech_delta_chain_stream(shape: TechHourCShape) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut seq = 1u64;
    fn push(
        stream: &mut Vec<u8>,
        seq: &mut u64,
        hour: i64,
        domain: u16,
        rtype: u16,
        payload: &[u8],
    ) {
        push_record(stream, *seq, raw_for(hour), domain, rtype, payload);
        *seq += 1;
    }
    fn push_roster(stream: &mut Vec<u8>, seq: &mut u64, hour: i64) {
        push(
            stream,
            seq,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(1, "AAA"), (2, "BBB"), (3, "CCC")]),
        );
        push(
            stream,
            seq,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(),
        );
    }

    push_roster(&mut stream, &mut seq, HOUR_A);
    push(
        &mut stream,
        &mut seq,
        HOUR_A,
        TECH_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &tech_payload(&[(1, 10, 1), (2, 20, 2), (3, 30, 3)], 0),
    );
    push(
        &mut stream,
        &mut seq,
        HOUR_A,
        TECH_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    push(
        &mut stream,
        &mut seq,
        HOUR_A,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(2, 2, 0),
    );

    push_roster(&mut stream, &mut seq, HOUR_B);
    push(
        &mut stream,
        &mut seq,
        HOUR_B,
        TECH_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &tech_payload(&[(1, 11, 1)], COMPACT_STATUS_DELTA_DATA),
    );
    push(
        &mut stream,
        &mut seq,
        HOUR_B,
        TECH_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload_with_status(COMPACT_STATUS_DELTA_DATA),
    );
    push(
        &mut stream,
        &mut seq,
        HOUR_B,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(2, 2, 0),
    );

    push_roster(&mut stream, &mut seq, HOUR_C);
    let (payload, status) = match shape {
        TechHourCShape::Patch => (
            tech_payload(&[(2, 77, 2)], COMPACT_STATUS_DELTA_DATA),
            COMPACT_STATUS_DELTA_DATA,
        ),
        TechHourCShape::WholePayload => (tech_payload(&[(1, 11, 1), (2, 77, 2)], 0), 0),
        TechHourCShape::PatchReassertingTheUnmeasuredCountry => (
            tech_payload(
                &[(1, 11, 1), (2, 77, 2), (3, 30, 3)],
                COMPACT_STATUS_DELTA_DATA,
            ),
            COMPACT_STATUS_DELTA_DATA,
        ),
        TechHourCShape::DivergentPatch => (
            tech_payload(&[(2, 111, 2)], COMPACT_STATUS_DELTA_DATA),
            COMPACT_STATUS_DELTA_DATA,
        ),
    };
    push(
        &mut stream,
        &mut seq,
        HOUR_C,
        TECH_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &payload,
    );
    push(
        &mut stream,
        &mut seq,
        HOUR_C,
        TECH_DOMAIN,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload_with_status(status),
    );
    push(
        &mut stream,
        &mut seq,
        HOUR_C,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(2, 2, 0),
    );

    stream
}

#[test]
fn the_d13_fixture_actually_publishes_research_into_the_folded_world() {
    let with_tech = fold_bytes(&tech_delta_chain_stream(TechHourCShape::WholePayload));
    let roster_only = {
        let mut stream = Vec::new();
        let mut seq = 1u64;
        for hour in [HOUR_A, HOUR_B, HOUR_C] {
            push_record(
                &mut stream,
                seq,
                raw_for(hour),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &country_identity_payload(&[(1, "AAA"), (2, "BBB"), (3, "CCC")]),
            );
            seq += 1;
            push_record(
                &mut stream,
                seq,
                raw_for(hour),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &perf_payload(),
            );
            seq += 1;
            push_record(
                &mut stream,
                seq,
                raw_for(hour),
                DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &frame_end_payload_with(1, 1, 0),
            );
            seq += 1;
        }
        fold_bytes(&stream)
    };
    assert_ne!(
        with_tech.hours[0].snapshot_hash, roster_only.hours[0].snapshot_hash,
        "D13's records put nothing in the folded world, so every equivalence assertion in \
         this module would pass over two empty worlds"
    );
    assert_eq!(
        with_tech.totals.malformed_payload_records, 0,
        "a D13 record was excluded as malformed before the fold saw it: {:?}",
        with_tech.totals.excluded_domain_data_records
    );
}

#[test]
fn the_d13_patch_stream_folds_identically_to_the_whole_payload_stream() {
    let patch = fold_bytes(&tech_delta_chain_stream(TechHourCShape::Patch));
    let whole = fold_bytes(&tech_delta_chain_stream(TechHourCShape::WholePayload));

    assert_eq!(
        patch.hours.len(),
        whole.hours.len(),
        "both producers publish the same three hours"
    );
    for (a, b) in patch.hours.iter().zip(whole.hours.iter()) {
        assert_eq!(
            a.hour_index, b.hour_index,
            "the two streams cover the same hours in the same order"
        );
        assert_eq!(
            a.snapshot_hash, b.snapshot_hash,
            "hour {} folds to a different world under D13's patch encoding: patch {} vs \
             whole {}",
            a.hour_index, a.snapshot_hash, b.snapshot_hash
        );
    }
}

#[test]
fn an_unmeasured_d13_country_is_retained_by_both_shapes() {
    let patch = fold_bytes(&tech_delta_chain_stream(TechHourCShape::Patch));
    let re_asserted = fold_bytes(&tech_delta_chain_stream(
        TechHourCShape::PatchReassertingTheUnmeasuredCountry,
    ));
    assert_eq!(
        patch.hours[2].snapshot_hash, re_asserted.hours[2].snapshot_hash,
        "hour C's patch dropped the unmeasured country 3: it folds to a different world than \
         the stream that re-asserts country 3 at the value hour A gave it"
    );
    assert_ne!(
        patch.hours[1].snapshot_hash, patch.hours[2].snapshot_hash,
        "hour C changed nothing, so this gate compares two identical worlds"
    );
}

#[test]
fn the_d13_fold_equivalence_gate_can_tell_two_worlds_apart() {
    let world = fold_bytes(&tech_delta_chain_stream(TechHourCShape::Patch));
    let other = fold_bytes(&tech_delta_chain_stream(TechHourCShape::DivergentPatch));

    assert_eq!(
        world.hours[0].snapshot_hash, other.hours[0].snapshot_hash,
        "the two streams are identical up to hour C, so hour A must still match"
    );
    assert_ne!(
        world.hours[2].snapshot_hash, other.hours[2].snapshot_hash,
        "a patch carrying a different value for country 2 folded to the same world"
    );
}

fn two_hours_with_one_excluded_payload(malformed_hour: i64) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for hour in [HOUR_A, HOUR_B] {
        let exact = hour != malformed_hour;
        let mut payload = country_identity_payload(&[(47, "GER")]);
        if !exact {
            payload.push(0xAA);
        }
        push_record(
            &mut stream,
            sequence,
            raw_for(hour),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        sequence += 1;
        push_record(
            &mut stream,
            sequence,
            raw_for(hour),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(),
        );
        sequence += 1;
        push_record(
            &mut stream,
            sequence,
            raw_for(hour),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
        );
        sequence += 1;
    }
    stream
}

#[test]
fn an_excluded_domain_data_payload_makes_its_hour_incomplete_for_that_domain() {
    let stream = two_hours_with_one_excluded_payload(HOUR_B);
    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect(
        "an excluded payload is ACCOUNTED, not fatal: the hour still seals and the capture is \
         never destroyed (owner ruling 'partial data still seals')",
    );

    assert_eq!(run.totals.excluded_domain_data_records.len(), 1);
    let excluded = &run.totals.excluded_domain_data_records[0];
    assert_eq!(excluded.domain_id, 18);
    assert_eq!(excluded.game_hour_raw, raw_for(HOUR_B));
    assert!(!excluded.validation_reason.is_empty());

    assert_eq!(
        run.hours
            .iter()
            .map(|hour| hour.hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_A, HOUR_B],
        "both hours seal; the exclusion degrades one domain, not the capture"
    );

    let hour_b = &run.hours[1].completeness;
    let hour_b_domain = hour_b
        .domains
        .get(&18)
        .expect("the excluded record is still a counted, well-framed record");
    assert_eq!(
        hour_b_domain.data_records, 1,
        "the record was framed and counted; it is its PAYLOAD that never reached the fold"
    );
    assert_eq!(hour_b_domain.perf_records, 1);
    assert_eq!(
        hour_b_domain.gap_records, 0,
        "an excluded payload is not a DOMAIN_GAP; the collector never declared an absence"
    );
    assert_eq!(hour_b_domain.normalized_status_flags_or, 0);
    assert_eq!(hour_b_domain.non_tolerated_status_flags, 0);
    assert_eq!(
        hour_b_domain.non_tolerated_data_payload_status_flags, 0,
        "the payload did not decode, so it yielded no status word to classify"
    );
    assert!(
        hour_b.declared_degraded_domains.is_empty(),
        "the H4FD notice is minted from the PERF status, which is clean, so no notice exists"
    );

    let report_b = completeness_json(hour_b);
    let reported_b = &report_b["domains"]["18"];
    assert_eq!(
        reported_b["complete"],
        serde_json::json!(false),
        "hour B lost domain 18's payload to the exclusion ledger, so the hour must not report \
         that domain complete: {report_b:#}"
    );
    assert_eq!(
        reported_b["state"],
        serde_json::json!("excluded"),
        "the state must name the record-level loss, not a status word the payload never \
         produced: {report_b:#}"
    );
    assert_eq!(
        reported_b["excluded_data_records"],
        serde_json::json!(1),
        "{report_b:#}"
    );
    let reasons = reported_b["excluded_data_reasons"]
        .as_array()
        .unwrap_or_else(|| panic!("the incompleteness must name its cause: {report_b:#}"));
    assert_eq!(reasons.len(), 1, "{report_b:#}");
    assert_eq!(
        reasons[0],
        serde_json::json!(excluded.validation_reason),
        "the report must carry the LEDGER's reason string through verbatim, not a restatement \
         of it: {report_b:#}"
    );
}

#[test]
fn an_excluded_payload_is_never_charged_to_a_later_hour() {
    let stream = two_hours_with_one_excluded_payload(HOUR_A);
    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect("the stream seals");

    assert_eq!(run.totals.excluded_domain_data_records.len(), 1);
    assert_eq!(
        run.totals.excluded_domain_data_records[0].game_hour_raw,
        raw_for(HOUR_A),
        "the ledger records the hour the RECORD carried in its envelope"
    );

    let report_a = completeness_json(&run.hours[0].completeness);
    let reported_a = &report_a["domains"]["18"];
    assert_eq!(
        reported_a["complete"],
        serde_json::json!(false),
        "{report_a:#}"
    );
    assert_eq!(
        reported_a["excluded_data_records"],
        serde_json::json!(1),
        "{report_a:#}"
    );

    let report_b = completeness_json(&run.hours[1].completeness);
    let reported_b = &report_b["domains"]["18"];
    assert_eq!(
        reported_b["complete"],
        serde_json::json!(true),
        "hour B's payload decoded and folded; charging it with hour A's exclusion is hour \
         misattribution: {report_b:#}"
    );
    assert_eq!(
        reported_b["excluded_data_records"],
        serde_json::json!(0),
        "{report_b:#}"
    );
    assert_eq!(
        reported_b["excluded_data_reasons"],
        serde_json::json!([]),
        "{report_b:#}"
    );

    assert_ne!(
        run.hours[0].completeness, run.hours[1].completeness,
        "an hour that lost a payload and an hour that did not must not compare equal"
    );
}

#[test]
fn an_exclusion_in_a_refused_hour_is_accounted_by_the_refusal_not_the_next_hour() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;

    let mut malformed = country_identity_payload(&[(47, "GER")]);
    malformed.push(0xAA);
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &malformed,
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal_payload(HOUR_A, 18),
    );
    sequence += 1;

    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );

    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect("an exclusion in a refused hour is accounted by the refusal, not a certification bar");

    assert_eq!(run.totals.excluded_domain_data_records.len(), 1);
    assert_eq!(
        run.totals.excluded_domain_data_records[0].game_hour_raw,
        raw_for(HOUR_A)
    );
    assert!(
        run.refusals.contains_key(&HOUR_A),
        "hour A is accounted as refused"
    );
    assert_eq!(
        run.hours
            .iter()
            .map(|hour| hour.hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_B],
        "only hour B published"
    );

    let report_b = completeness_json(&run.hours[0].completeness);
    let reported_b = &report_b["domains"]["18"];
    assert_eq!(
        reported_b["complete"],
        serde_json::json!(true),
        "hour B folded its own exact payload; the refused hour's exclusion is not its loss: \
         {report_b:#}"
    );
    assert_eq!(
        reported_b["excluded_data_records"],
        serde_json::json!(0),
        "{report_b:#}"
    );
}

#[test]
fn the_ledger_walk_skips_a_refused_hours_exclusion_and_still_takes_the_current_ones() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    let mut malformed = country_identity_payload(&[(47, "GER")]);
    malformed.push(0xAA);

    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &malformed,
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal_payload(HOUR_A, 18),
    );
    sequence += 1;

    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &malformed,
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload(),
    );
    sequence += 1;
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with(1, 1, 0),
    );

    let run = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &one_domain_roster(),
    )
    .expect("both losses are accounted, so the stream still seals");

    assert_eq!(
        run.totals.excluded_domain_data_records.len(),
        2,
        "one exclusion in the refused hour, one in the published hour"
    );
    assert_eq!(
        run.hours
            .iter()
            .map(|hour| hour.hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_B]
    );

    let report = completeness_json(&run.hours[0].completeness);
    let reported = &report["domains"]["18"];
    assert_eq!(
        reported["excluded_data_records"],
        serde_json::json!(1),
        "hour B owns exactly its OWN loss — not zero, and not both: {report:#}"
    );
    assert_eq!(
        reported["state"],
        serde_json::json!("excluded"),
        "{report:#}"
    );
}

#[test]
fn the_production_expected_roster_never_demands_a_passive_refused_domain() {
    let roster = ExpectedRoster::production();
    for id in compact_contract::PASSIVE_PRODUCTION_REFUSED_DOMAIN_IDS {
        assert!(
            !roster.domains.contains_key(id),
            "domain {id} is refused by the shipped passive product at attach; \
             certification must not demand it of a passive capture"
        );
    }
    assert!(
        !roster.domains.is_empty(),
        "the filter must narrow the profile, never empty it"
    );
}
