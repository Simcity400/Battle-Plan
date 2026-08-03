//! In-memory unit tests for the fold-equivalence gate. They build a small
//! synthetic H4CR stream (via the same wire layout the collector emits), fold it
//! through the production `WorldFold`, and assert stable hashing, PERF-record
//! ignoring, mutation detection, and fail-closed certification.

use super::*;
use compact_contract::calendar::DATE_EPOCH_CONST;
use compact_contract::campaign_identity::{
    encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses,
    COMPACT_DOMAIN_STREAM_HEADER, COMPACT_RECORD_TYPE_STREAM_HEADER,
};
use compact_contract::{
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_SOURCE_UNAVAILABLE,
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

// --- synthetic stream builders -------------------------------------------

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

/// country_identity (domain 18): u64 wall, u32 row_count, u32 changed, u32
/// status, then per row: u32 tag_db_id, u16-len TAG, u8 cosmetic marker (1=null).
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
        p.push(1); // cosmetic null
    }
    p
}

/// An exact base DOMAIN_PERF payload that the fold must ignore entirely.
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

/// Three complete game-hours. Hour B carries a PERF record (must be ignored) and
/// hour C a second country. Returns the stream and the byte offset of the first
/// `tag_db_id` in hour A's domain-18 payload (a real payload byte, for mutation).
fn synthetic_stream() -> (Vec<u8>, usize) {
    let mut s = Vec::new();
    let mut seq = 1u64;

    // Hour A: one country, its owned PERF record, then frame_end.
    let a_payload = country_identity_payload(&[(47, "GER")]);
    // header (32) + payload prefix: u64 wall(8)+u32 row(4)+u32 changed(4)+u32 status(4) = 20
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

    // Hour B: same country, plus a PERF record that must not affect the fold.
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

    // Hour C: two countries and the same owned PERF coverage.
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

/// A `DOMAIN_GAP` (`H4DG`) payload whose witnesses prove the closed
/// `ClockUnreadable` reason for exactly `hour_index`.
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

/// The same absence for a different closed reason: the domain's worker panicked.
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

/// A `FRAME_REFUSED` (`H4FR`) payload naming one offending domain.
fn refusal_payload(hour_index: i64, domain_id: u16) -> Vec<u8> {
    compact_contract::frame_refusal::FrameRefusalRecordV1 {
        refused_hour: raw_for(hour_index),
        offenders: vec![compact_contract::frame_refusal::FrameRefusalOffender {
            domain_id,
            status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            reason: "synthetic_offender=1".to_string(),
        }],
    }
    .encode()
    .expect("refusal names an offender")
}

/// The externally authoritative roster used by the record-shape tests: a single
/// domain that must produce one PERF and one DATA record per hour.
fn one_domain_roster() -> ExpectedRoster {
    ExpectedRoster::test(&[(18, true)])
}

// --- tests ----------------------------------------------------------------

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
    assert_eq!(chain_hash(&identity_run.hours), chain_hash(&legacy_run.hours));
    assert_eq!(identity_run.totals.data_records, legacy_run.totals.data_records);
    assert_eq!(identity_run.totals.perf_records, legacy_run.totals.perf_records);
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
    assert!(error.contains("identity-v1 stream validation at byte 0"), "{error}");
    assert!(error.contains("invalid CID1 stream header"), "{error}");
}

#[test]
fn identity_mode_cli_is_explicit_and_defaults_to_legacy() {
    assert_eq!(
        parse_stream_identity_mode(&[]).unwrap(),
        StreamIdentityMode::Legacy
    );
    assert_eq!(
        parse_stream_identity_mode(&[
            "--identity-mode".to_string(),
            "identity-v1".to_string(),
        ])
        .unwrap(),
        StreamIdentityMode::IdentityV1
    );
    assert!(parse_stream_identity_mode(&[
        "--identity-mode".to_string(),
        "auto".to_string(),
    ])
    .is_err());
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
    // A stream identical to the synthetic one but WITHOUT hour B's PERF record
    // must fold to the identical per-hour hashes: the PERF record carries no
    // folded state.
    let (with_perf, _) = synthetic_stream();
    let run_with = fold_bytes(&with_perf);
    assert_eq!(run_with.totals.perf_records, 3, "PERF records were seen");

    // Rebuild without the perf record.
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

    // Both FRAME_END counts agree with each other and with the external roster
    // size, but domain 26 is absent from the actual per-hour DATA/PERF records.
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
        compact_contract::production_profile_domains().len()
    );
    assert!(production.domains.contains_key(&18));
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

    // Flip one bit of a real DATA payload byte (hour A's first tag_db_id).
    let mut mutated = stream.clone();
    mutated[offset] ^= 0x01;
    let after = fold_bytes(&mutated);

    // Same hour range, but hour A now folds to a different hash while B and C
    // (untouched) stay identical — exactly what `compare` keys off.
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
    // Chop the last 5 bytes → the final frame_end record is incomplete.
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
    // The terminal-boundary rule is asserted in the SHARED authority's own
    // words. `fold_check` used to restate it ("final record must be canonical
    // FRAME_END") inside its completeness pass; that was the third independent
    // copy of a rule whose second copy had already cost a real bug, and it was
    // NARROWER than the shared rule — it named FRAME_END alone, while
    // `BoundaryTracker::eof_check` also admits an honestly-refused stream that
    // ends on FRAME_REFUSED. Pinning the shared wording is what stops the
    // duplicate coming back: this assertion can only pass if the verdict
    // arrived through `strict_stream_certification_verdict`.
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

// --- DOMAIN_GAP and FRAME_REFUSED --------------------------------------------
//
// Both record types reach this tool on EVERY real capture on disk: each of the
// three collector streams captured on 2026-07-31 carries exactly 73 DOMAIN_GAP
// records and between 254 and 449 FRAME_REFUSED records. The completeness audit
// used to reach `unreachable!()` on both, so `fold_check` panicked rather than
// reaching any verdict at all.

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
    // The verdict has to say what actually happened. "PERF record count 1,
    // observed 0" would send a reader hunting for a record that was never
    // supposed to exist: the gap IS the terminal outcome, and it carries the
    // reason the collector recorded.
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
    // Two hours identical in every published byte: the same DOMAIN_DATA for
    // domain 18, the same seal, and domain 26 absent in both. They differ only
    // in the REASON domain 26 was absent -- an unreadable clock in one, a
    // panicked worker in the other. Those are different facts about the hour,
    // and a certification that cannot tell them apart is not certifying the
    // absence at all.
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
        clock.hours[0].snapshot_hash, panic.hours[0].snapshot_hash,
        "the folded state is identical: a gap folds nothing in either stream"
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
    // Hour B is attempted -- it produces a terminal outcome for domain 18 --
    // and then refused. Nothing about hour B is published.
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
}

#[test]
fn a_stream_that_publishes_no_hour_certifies_nothing() {
    // Every real capture on disk has this shape at the tail: hour after hour
    // attempted and refused, nothing published. Reporting `hours: []` with the
    // SHA-256 of the empty chain and exit 0 would be a certifier that accepts a
    // stream containing no extracted data at all.
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

    // The published hours are byte-identical between the two streams, so a
    // fold-equivalence check over snapshots alone would call them the same.
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

    // `compare` therefore has to report a divergence, or it certifies two
    // different capture outcomes as the same run.
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
    // The defect was a PREMISE, not a typo: the audit's fallback arm was
    // `unreachable!("record semantics rejected unknown type")`, asserting that
    // `validate_compact_record_semantics` admits only DATA, PERF and FRAME_END.
    // It also admits DOMAIN_GAP and FRAME_REFUSED, so the arm was reachable --
    // and reached on the very first record of every real capture.
    //
    // Re-testing the two types that were missed would only re-test today's
    // omission. This asserts the invariant the premise got wrong: whatever the
    // contract admits, the audit accounts. Sweeping the whole u16 space the
    // contract could ever open keeps the two authorities from drifting apart
    // again.
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
            let outcome = audit.observe(raw_for(HOUR_A), domain_id, record_type, 0, &payload);
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
    // Checked in both directions on purpose. The audit keeps published hours
    // and refused hours in separate maps, so a one-sided guard would let the
    // same hour sit in both and hand the published account to the caller as if
    // the refusal had not happened.
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
            .observe(raw_for(HOUR_A), ordered[0].0, ordered[0].1, 0, &ordered[0].2)
            .expect("the first close of the hour is legitimate");
        let err = audit
            .observe(raw_for(HOUR_A), ordered[1].0, ordered[1].1, 0, &ordered[1].2)
            .expect_err("the same hour cannot be closed twice with contradictory outcomes");
        assert!(
            err.contains("is both"),
            "refuse_first={refuse_first}: {err}"
        );
    }
}

#[test]
fn compare_reports_a_divergence_when_only_the_refusals_differ() {
    // The helper being right is not the same as `compare` asking it. This
    // drives the whole decision `compare` reports `identical` from, so
    // dropping the refusal check out of that decision -- not just breaking the
    // helper -- turns this red.
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
    let run_b = run_input(&path_b, &roster).expect("published hour A certifies").0;
    let run_c = run_input(&path_c, &roster).expect("published hour A certifies").0;

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
    // The decoder feeds every DOMAIN_DATA straight into the production fold and
    // only drains on FRAME_END, so a FRAME_REFUSED does not discard what the
    // refused hour had already ingested. The refused hour's record therefore
    // reaches the NEXT published hour's fold. `fold_check` must not certify
    // that, because the published hour would carry rows the collector
    // deliberately withheld.
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    // Hour A is attempted with one DOMAIN_DATA, then refused.
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
    // Hour B publishes one DOMAIN_DATA of its own.
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

    // This test was written when only the AUDIT had stopped bleeding a refused
    // hour into the next one, so it asserted the disagreement that exposed the
    // fold still doing it: `domain 18 production fold 2 vs completeness audit 1`.
    // `decoder_carryover` then fixed the fold side too, which voids that
    // premise -- both sides now agree, and a test demanding they disagree would
    // fail for the RIGHT reason while reading as a regression. What the name
    // always claimed is asserted directly instead: hour B publishes exactly the
    // one record that belongs to it.
    assert_eq!(
        run.hours
            .iter()
            .map(|hour| hour.hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_B],
        "hour A was refused, so hour B is the only published hour"
    );
    // Per HOUR, not stream-wide: the stream really does carry two domain-18
    // DATA records, one in each hour. Carry-over is a question of which hour
    // each is ATTRIBUTED to, so the stream total cannot answer it.
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
    // One capture on disk refuses 449 consecutive hours, each naming up to two
    // dozen offenders with a typed reason string. Rendering every one turns the
    // report into megabytes of near-identical text; rendering a prefix without
    // saying so would understate how much of the capture was refused. The
    // per-hour DETAIL is bounded; the count, the span and the offending domains
    // are exact.
    let mut refusals: BTreeMap<i64, HourRefusal> = BTreeMap::new();
    let total = REFUSAL_DETAIL_LIMIT + 17;
    for step in 0..total {
        refusals.insert(
            HOUR_A + step as i64,
            HourRefusal {
                offenders: vec![RefusalOffender {
                    domain_id: 18 + (step % 3) as u16,
                    status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    reason: "synthetic_offender=1".to_string(),
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
    // `WorldFold` decides whether to apply a domain's rows from the status
    // EMBEDDED IN THE DATA PAYLOAD -- `fold_country_identity` reads it at
    // payload offset 16 and returns without folding a single row when it is a
    // failure. Certification read only the matching DOMAIN_PERF record. A
    // payload saying "the read did not complete" underneath a PERF saying
    // "clean" therefore certified as a complete hour whose domain had in fact
    // contributed nothing, and the report said `data_records: 1` with a clean
    // status.
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
            // The PERF record is CLEAN. It is the only status the certifier
            // used to read.
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

    // The clean control certifies, so the refusal below is about the status and
    // nothing else about the stream shape.
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
        err.contains("DOMAIN_DATA payload status"),
        "the verdict must name the payload's own status, not the PERF's: {err}"
    );
    assert!(
        err.contains("non-tolerated failure flags"),
        "{err}"
    );
}

#[test]
fn the_data_payload_status_is_part_of_the_hours_certified_identity() {
    // A tolerated payload status must still be visible. Two hours whose only
    // difference is a status the fold tolerates produce the same rows, so the
    // snapshots match; the certified identity must not.
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

    // WHICH record carried the status is itself a fact about the hour. These
    // two streams OR to the same per-domain status -- one carries the flag on
    // the PERF record only, the other on both -- so a certification that folds
    // the payload status into the PERF status and keeps nothing else cannot
    // tell them apart. That is the disagreement this field exists to expose.
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

// --- degraded-frame certification ------------------------------------------
//
// Until this record existed, frame completeness was all-or-nothing: one domain
// with a non-tolerated failure refused the whole frame, so across three walked
// live captures the stream never carried a single DOMAIN_DATA record and no
// domain fix could be verified end to end. These tests pin the other outcome --
// the frame publishes and the failing domains are named unknown -- and, more
// importantly, they pin the boundary that keeps that honest: a degradation the
// stream does not DECLARE is still not certifiable.

/// Build one hour: a clean DATA record, one PERF record per `(domain, status)`
/// pair, optionally an `H4FD` notice declaring `declared`, then `FRAME_END`.
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
        let notice = compact_contract::frame_degraded::FrameDegradedRecordV1 {
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
                        reason: (*reason).to_string(),
                    }
                })
                .collect(),
        };
        // Encoded through the real codec on purpose: a notice the contract
        // would refuse must not be constructible for a test either.
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
    // The FRAME_END counts must equal the number of terminal domain outcomes.
    // Note what is NOT counted here: the H4FD notice. A degraded notice is not
    // a terminal outcome for any domain -- the domain still emitted its own
    // PERF record -- and the boundary tracker agrees, which is why this count
    // is the PERF count and not the PERF count plus one.
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

/// A notice may not name a domain its status does not support. Otherwise the
/// two halves of the wire disagree, and a consumer trusting one reads a
/// different frame from a consumer trusting the other.
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
    // Self-consistent as a record, but domain 30's PERF above is clean.
    let notice = compact_contract::frame_degraded::FrameDegradedRecordV1 {
        degraded_hour: raw_for(HOUR_A),
        domains: vec![compact_contract::frame_degraded::FrameDegradedDomain {
            domain_id: 30,
            status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
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

/// THE deliverable. An hour with failing domains PUBLISHES and certifies, and
/// the certified hour records exactly those domains as unknown.
///
/// Domains 29 and 30 have no `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry, so
/// their failure bits are genuine untolerated defects -- the exact shape that
/// refused every frame in every live capture before this change.
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

    // Published: the domain that read fine really is in the certified hour.
    // Before this change the whole frame was withheld for the other two.
    assert_eq!(
        hour.domain_data_counts.get(&18),
        Some(&1),
        "the domains that read fine must publish; that is the entire point"
    );

    // And the ones that failed are certified UNKNOWN, with their exact bits.
    assert_eq!(
        hour.completeness.declared_degraded_domains,
        [
            (29u16, COMPACT_STATUS_PARTIAL_DATA),
            (30u16, COMPACT_STATUS_SOURCE_UNAVAILABLE)
        ]
        .into_iter()
        .collect::<BTreeMap<u16, u32>>(),
        "a degraded domain must be certified as unknown, never as complete"
    );
    assert!(
        !hour.completeness.declared_degraded_domains.contains_key(&18),
        "a clean domain must never appear in the degraded declaration"
    );
}

/// The notice must belong to the hour it rides in, or a consumer would mark one
/// hour unknown while a different hour carried the data.
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
    // Rewrite only the payload's `degraded_hour`. The envelope hour is left
    // alone, so nothing but the self-agreement check can catch this.
    let magic = compact_contract::frame_degraded::FRAME_DEGRADED_MAGIC;
    let at = stream
        .windows(4)
        .position(|window| window == magic)
        .expect("the stream carries an H4FD payload");
    stream[at + 8..at + 12].copy_from_slice(&raw_for(HOUR_B).to_le_bytes());
    let err = collect_run(Cursor::new(stream.clone()), stream.len() as u64)
        .err()
        .expect("a notice framed for another hour must not certify");
    // Asserted against the boundary tracker's wording specifically ("at byte"),
    // because that is the single owner of this rule. If the verdict ever starts
    // coming from somewhere else, a second copy has appeared and this assertion
    // is how it gets noticed.
    assert!(
        err.contains("FRAME_DEGRADED") && err.contains("at byte"),
        "the boundary tracker must be the layer that catches this: {err}"
    );
}

/// An expected-partial is NOT a degradation and must not be declared as one.
/// This is the constraint the record exists to protect, checked at the
/// certifier and not only in the codec.
#[test]
fn a_tolerated_partial_certifies_and_is_never_declared_degraded() {
    // Domain 26's tolerance covers exactly PARTIAL_DATA.
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

    // And declaring it degraded is refused by the codec, so a stream cannot
    // even be BUILT that way.
    assert!(
        compact_contract::frame_degraded::FrameDegradedDomain::from_status(
            26,
            COMPACT_STATUS_PARTIAL_DATA,
            "mislabelled",
        )
        .is_none(),
        "a tolerated partial must not produce a degraded entry"
    );
}

/// The non-weakening proof, and the mutation target for the whole deferral.
///
/// The certifier used to reject a non-tolerated PERF failure the instant it saw
/// one. That verdict is unchanged for an UNDECLARED failure -- deferring the
/// check to FRAME_END buys the ability to tell a declared defect from a hidden
/// one, and buys nothing else.
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
