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
    //
    // The VERDICT now comes from the FRAME_END reconciliation rather than from
    // the DOMAIN_DATA arm itself: the payload's residual joins the PERF's in the
    // observed-degraded set, and an hour whose notice does not account for it
    // refuses there. The refusal is unchanged; only its owner is, and there is
    // now exactly one. What that buys is the case this test cannot see and t26
    // is made of -- a payload failure the frame DOES declare, which the local
    // refusal rejected while the identical word in a PERF certified.
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
        err.contains("domain 18"),
        "the verdict must name the offending domain: {err}"
    );
    assert!(
        err.contains("DOMAIN_DATA"),
        "the verdict must name the payload's own status, not the PERF's: {err}"
    );
    assert!(
        err.contains("non-tolerated failure flags"),
        "{err}"
    );
    assert!(
        err.contains("declares no H4FD degraded notice"),
        "the payload failure must refuse for being UNDECLARED, which is the fact \
         that distinguishes it from t26's accounted d86: {err}"
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

    // Nor may it be laundered in the other direction. `complete` and
    // `expected-partial` are distinct states in the shared vocabulary
    // (`compact_contract::frame_degraded`): a tolerated partial's rows are a
    // reviewed SPARSE patch, so a report that renders it as complete is telling
    // a consumer the hour holds rows it never carried.
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
    // The control: the untouched domain in the same hour IS complete, so the
    // assertion above is not simply reporting that nothing is ever complete.
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

    // The same rule reached through the OTHER record. A domain whose PERF is
    // clean while its own DOMAIN_DATA payload carries a tolerated partial gets
    // no H4FD notice (the notice is minted from the PERF) and clears the
    // FRAME_END payload-coverage loop (its residual is zero), so the status
    // classification is the only thing standing between it and a false
    // "complete". Asserted directly on the classifier because building a real
    // domain-26 DATA payload means reproducing that domain's grammar; the rule
    // under test is the classification, not the wire shape.
    // The field shape is exactly what `CompletenessAudit::observe` builds for a
    // clean PERF plus a DATA payload carrying PARTIAL_DATA: the payload's word
    // in `data_payload_status_flags_or`, the same bit unioned into
    // `normalized_status_flags_or`, and BOTH residuals zero because domain 26
    // tolerates that bit.
    let payload_side_partial = DomainCompleteness {
        data_records: 1,
        perf_records: 1,
        data_payload_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
        normalized_status_flags_or: COMPACT_STATUS_PARTIAL_DATA,
        ..DomainCompleteness::default()
    };
    // The PREMISE, asked of the contract rather than restated from the struct
    // literal above: domain 26 tolerates this bit completely, so BOTH residuals
    // really are zero for this shape and no residual can be what makes the
    // domain incomplete. If that tolerance entry were ever removed, this fires
    // here rather than silently turning the assertions below into a test of the
    // degraded path.
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
    // Domain 18 has no tolerance entry, so the identical bit there is a
    // non-tolerated failure and a different state. This proves the assertion
    // above reads the tolerance table rather than merely reacting to the bit.
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

/// Which state a D86 producer-declared partial maps to, decided once and
/// pinned here.
///
/// Owner ruling 2026-08-06: replay ADMITS a d86 history whose producer declared
/// `producer_complete = false`, carrying the rows it has with the loss named.
/// The hour that delivery belongs to is then `expected_partial` and nothing
/// else in the shared vocabulary:
///
///  * not `complete` — a sinking IS missing, and the SHP1 loss block says how
///    many and why;
///  * not `degraded_unknown` — that state means the rows must NOT be applied
///    and the accumulator must be retained, which is the wholesale refusal the
///    ruling removed;
///  * not `absent` — the domain published a `DOMAIN_DATA` record, not a
///    `DOMAIN_GAP`;
///  * not `excluded` — the decoder's payload authority admits the record.
///
/// `expected_partial` is the state the vocabulary already defines for exactly
/// this: a reviewed partial whose rows are a sparse patch and whose omissions
/// are known and not fabricated as deletions. The written integrity argument
/// lives in the domain-86 `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry, as that
/// state requires; no new spelling was invented.
///
/// SCOPE. This test pins the CLASSIFICATION only: given the status word a
/// producer-declared partial carries, `state()` answers `expected_partial`. It
/// builds `DomainCompleteness` by hand -- the same way the domain-26 fixture
/// above does, and for the same reason -- so it does NOT cover the carrier
/// actually emitting that word. That half is pinned in the lane, by
/// `a_partial_receipt_declares_partial_data_and_a_complete_one_declares_nothing`
/// and `a_receipt_status_word_that_contradicts_its_loss_block_is_refused`, and
/// end to end by `a_cd_style_partial_batch_certifies_without_a_degraded_notice`
/// below, which certifies a real stream built from a real partial payload.
///
/// What production change would make this fail: removing the domain-86
/// tolerance entry, which makes this word `degraded_unknown`; or changing
/// `DomainCompleteness::state` so a tolerated partial reports `complete`.
#[test]
fn a_d86_producer_declared_partial_hour_certifies_as_expected_partial() {
    // The premise, asked of the contract: domain 86 tolerates PARTIAL_DATA
    // outright, so neither residual can be what decides the state below.
    assert_eq!(
        compact_status_failure_flags_after_expected_partial_tolerance(
            86,
            COMPACT_STATUS_PARTIAL_DATA,
        ),
        0,
        "domain 86 must tolerate PARTIAL_DATA outright"
    );
    // The shape `CompletenessAudit::observe` builds for d86: the PERF word is
    // clean (the collector schedules the domain normally), and the partiality
    // arrives only in the DOMAIN_DATA payload's own SHP1 status word.
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

    // The control: the same domain with a clean payload is complete, so the
    // assertion above is reading the declaration and not the domain id.
    assert_eq!(
        DomainCompleteness {
            data_records: 1,
            perf_records: 1,
            ..DomainCompleteness::default()
        }
        .state(86),
        "complete"
    );

    // And the partial can never be written down as a degradation.
    assert!(
        compact_contract::frame_degraded::FrameDegradedDomain::from_status(
            86,
            COMPACT_STATUS_PARTIAL_DATA,
            "mislabelled",
        )
        .is_none(),
        "a tolerated d86 partial must not produce a degraded entry"
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

// --- d86: a payload's own status is reconciled, not separately refused ------
//
// The production profile refused run t26 at byte 7673 on domain 86's
// DOMAIN_DATA payload status `0x00000100` -- `source_unavailable`, an assigned
// v1 failure bit meaning the read did not get through. That bit is genuinely
// failure-class and must NOT be added to `COMPACT_EXPECTED_PARTIAL_TOLERANCES`:
// a tolerance entry asserts a reviewed partial with a written integrity
// argument, and "the source could not be read" is the opposite of that.
//
// What t26 actually carries is the ACCOUNTED shape: the same word in d86's
// PERF, and an `H4FD` notice naming domain 86 with exactly that bit and the
// reader's typed reason. The fold withholds d86's rows and retains its
// accumulator -- precisely what the notice declares -- so the hour certifies
// with the domain recorded unknown, which is what the partial-data law asks
// for. The defect was that the DOMAIN_DATA arm answered that question a second
// time, and more strictly, than the FRAME_END reconciliation that owns it: on
// those very hours d18, d47, d53, d60, d85 and d90 certified as
// declared-degraded carrying the same or wider failure words.

/// One SHP1 payload built through the lane's own encoder.
///
/// `producer_loss` present means a CD-style PARTIAL BATCH: rows that survived,
/// travelling beside a named loss, with `producer_complete` false.
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
        // The lane refuses an unavailable frame that carries a snapshot and a
        // snapshot-bearing frame that carries a status, so an unavailable
        // payload is the only shape a failing d86 read can publish.
        return d86::encode_unavailable(hour_index as u64, 7, status_flags)
            .expect("an unavailable d86 payload encodes");
    }
    // A receipt frame's status word is DERIVED from its loss block, exactly as
    // `collect_and_encode` derives it: a producer-declared partial says so in
    // the shared status vocabulary too, or the certifier cannot see it.
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

/// One hour holding exactly d86: its DATA payload, its PERF, an optional `H4FD`
/// notice, then a closing frame record.
///
/// `close` is what terminates the hour, so the same body can be published or
/// refused -- which is the only way to build the "declaration outlives its
/// hour" case at all.
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
        let notice = compact_contract::frame_degraded::FrameDegradedRecordV1 {
            degraded_hour: raw_for(hour_index),
            domains: vec![compact_contract::frame_degraded::FrameDegradedDomain {
                domain_id: 86,
                status_flags,
                non_tolerated_flags: compact_status_failure_flags_after_expected_partial_tolerance(
                    86,
                    status_flags,
                ),
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

/// THE t26 SHAPE. A `DOMAIN_DATA` payload whose own status carries a
/// non-tolerated failure certifies when -- and only when -- the frame's `H4FD`
/// notice accounts for exactly those bits.
///
/// The reason string is t26's verbatim d86 prefix, so the fixture is the
/// production shape and not a convenient invention.
///
/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`):
///   Reverting the DOMAIN_DATA arm to its immediate refusal
///   (`if residual != 0 { return Err(...) }` before the deferral) fails this
///   test with:
///     called `Result::unwrap` on an `Err` value: "completeness record at byte
///     0: domain 86 DOMAIN_DATA payload status 0x00000100 has non-tolerated
///     failure flags 0x00000100; the fold withholds this domain's rows on that
///     status, so the hour cannot certify as complete"
///
/// Production change that makes this fail: refusing a DOMAIN_DATA payload
/// status locally instead of reconciling it against the frame's declaration --
/// which is the state that refused t26 while `stream_fold` folded the same
/// corpus, and which would refuse every t27 hour whose d86 read did not get
/// through.
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
        Some(&COMPACT_STATUS_SOURCE_UNAVAILABLE),
        "the certified hour must record d86 as unknown with its exact bits"
    );
    // The word is still on the identity, so "accounted" never means "erased".
    assert_eq!(
        hour.completeness
            .domains
            .get(&86)
            .map(|domain| domain.data_payload_status_flags_or),
        Some(COMPACT_STATUS_SOURCE_UNAVAILABLE),
        "the payload's own status must remain in the certified evidence"
    );

    // And the accounting is VISIBLE in the emitted report, not merely held in
    // memory. A certified degraded hour that reads identically to a certified
    // clean hour would make "accounted" an unverifiable claim.
    //
    // Production change that makes this fail: dropping
    // `declared_degraded_domains` from `completeness_json`, which is the state
    // this report shipped in before the deferral made degraded hours reachable
    // through the DOMAIN_DATA path at all.
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

/// The non-weakening proof for the DOMAIN_DATA side, and the reason no
/// tolerance was widened: an UNNAMED payload failure still refuses.
///
/// This is the original finding restated by its owner. `WorldFold` decides
/// whether to apply a domain's rows from the status embedded in the payload, so
/// a payload declaring failure under a clean PERF makes the fold withhold every
/// row of an hour that would otherwise certify as complete.
///
/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): deleting the
/// DOMAIN_DATA arm's `domain.non_tolerated_status_flags |= residual;`
/// accumulation -- i.e. deferring the payload status to a reconciliation it
/// never reaches -- fails this test, and two more, with:
///     thread 'tests::an_undeclared_d86_payload_failure_still_refuses' panicked
///     at fold_check\src\tests.rs:2385:6:
///     an unnamed d86 payload failure must never certify
/// (`a_d86_declaration_with_the_wrong_bits_refuses` and
/// `a_data_payload_that_declares_failure_cannot_certify_behind_a_clean_perf`
/// fail on the same mutation; 49 passed, 3 failed.)
///
/// Production change that makes this fail: dropping the payload's residual out
/// of the observed-degraded set, which would restore exactly the hole the
/// payload-status check was added to close.
#[test]
fn an_undeclared_d86_payload_failure_still_refuses() {
    let stream = d86_hour_stream(
        &sunk_ship_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE, None),
        // The PERF is CLEAN, so nothing but the payload says anything is wrong
        // and the collector would mint no notice.
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

/// A declaration that does not MATCH the payload's bits is a disagreement, not
/// an accounting, and must refuse. Otherwise "declared degraded" would be a
/// blanket amnesty for whatever the payload happened to carry.
///
/// Production change that makes this fail: reconciling on domain identity
/// instead of on the exact bits.
#[test]
fn a_d86_declaration_with_the_wrong_bits_refuses() {
    let stream = d86_hour_stream(
        &sunk_ship_payload(
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
            None,
        ),
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        Some((COMPACT_STATUS_SOURCE_UNAVAILABLE, "domain 86: read_completion=2")),
    );
    let err = collect_certified_run(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        &d86_roster(),
    )
    .err()
    .expect("a declaration that understates the payload's bits must not certify");
    assert!(err.contains("domain 86 DOMAIN_DATA payload status"), "{err}");
    assert!(
        err.contains("does not declare") && err.contains("0x00000300"),
        "the verdict must show both words: {err}"
    );
}

/// The union of a domain's two residuals is not its degradation, and a notice
/// may not be satisfied by a word neither record carries.
///
/// This is the shape ORing the PERF and payload residuals admitted: `0x200` in
/// the PERF, `0x100` in the payload, a notice declaring `0x300`, and an hour
/// that certifies against a status no record of it ever published. The two
/// rules are therefore stated separately -- the notice EQUALS the PERF residual
/// and COVERS the payload residual -- and a union can satisfy neither.
///
/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): restoring the
/// union (`domain.non_tolerated_status_flags |= residual` in the DOMAIN_DATA
/// arm, and neutering the payload coverage loop) fails this test with:
///     thread 'tests::a_notice_may_not_be_satisfied_by_the_union_of_two_disagreeing_statuses'
///     panicked at fold_check\src\tests.rs:2507:6:
///     a notice declaring the union of two disagreeing words must not certify
/// -- i.e. the mutated build CERTIFIES the hour, which is Codex round-1
/// BLOCKER 2 reproduced.
///
/// Production change that makes this fail: merging the payload residual into
/// the PERF-derived `non_tolerated_status_flags` instead of keeping its own.
#[test]
fn a_notice_may_not_be_satisfied_by_the_union_of_two_disagreeing_statuses() {
    // Both words are non-tolerated for d86. PARTIAL_DATA no longer is -- it is
    // the domain's expected-partial declaration since the producer-declared
    // partial became an admitted delivery -- so using it here would make the
    // PERF residual zero and the test would pass for the wrong reason.
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

/// A refused frame's `H4FD` declaration must not survive into the next hour.
///
/// `H4FD` annotates a published hour; `H4FR` closes an hour that published
/// nothing. Holding the declaration across the refusal let the NEXT hour
/// inherit it, so a domain that failed there with no notice of its own
/// certified against the refused hour's notice -- certification of an unnamed
/// failure, reached through the record that closes hours.
///
/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): deleting the
/// `std::mem::take(&mut self.current_declared_degraded)` guard from the
/// FRAME_REFUSED arm fails this test with:
///     thread 'tests::a_refused_frames_declaration_does_not_certify_the_next_hours_failure'
///     panicked at fold_check\src\tests.rs:2566:6:
///     hour A's declaration must not certify hour B's undeclared failure
/// -- i.e. the mutated build CERTIFIES hour B, which is Codex round-1
/// BLOCKER 1 reproduced.
///
/// Production change that makes this fail: closing an hour without clearing the
/// declaration it accumulated.
#[test]
fn a_refused_frames_declaration_does_not_certify_the_next_hours_failure() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    // Hour A: a d86 failure, DECLARED, then refused rather than published.
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
    // Hour B: the SAME failure and NO notice of its own. Its only possible
    // accounting is hour A's, which died with hour A.
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

/// TOMORROW'S SHAPE (t27). Lane CD's partial batch -- rows that survived,
/// `producer_complete = false`, and a 48-byte loss block naming the cause --
/// certifies as an ORDINARY COMPLETE hour, with no `H4FD` notice and no
/// tolerance entry.
///
/// That is the correct outcome and not an oversight. A partial batch is a
/// successful read whose rows are all present and whose shortfall is stated in
/// the payload's own grammar; it publishes under
/// `AuthorityState::StaticExact20Receipt` with a clean status word, because
/// nothing about the READ failed. Only the three authority-denying refusal
/// codes (`EvidenceInstall`, `BaselineIncomplete`, `BaselineNotRevalidated`)
/// deny a batch, and those take the unavailable arm above -- which certifies
/// only when the frame declares it.
///
/// SCOPE, stated because it is narrower than the tests above and a reader
/// should not mistake it for one of them: this pins the CONTRACT BOUNDARY
/// between the d86 lane and the certifier, not the deferral rule. Its DATA and
/// PERF statuses are both clean, so reverting the deferral leaves it green --
/// deliberately. What it forbids is the other half of the alignment CD's change
/// requires: that a named partial batch must not become uncertifiable.
///
/// MUTATION RUN (verbatim, `cargo test -p fold_check --release`): setting
/// `producer_complete: true` while keeping the loss block fails this test in the
/// lane's own validator, before certification is reached:
///     thread 'tests::a_cd_style_partial_batch_certifies_without_a_degraded_notice'
///     panicked at fold_check\src\tests.rs:2208:6:
///     a d86 snapshot payload encodes: Invalid("SHP1 completeness claim and
///     producer loss disagree")
/// -- which is the check that stops a partial batch from presenting itself as
/// whole, and is why a clean status word here is honest.
///
/// Production change that makes this fail: deriving a failure status bit from
/// `producer_complete == false` without also emitting the matching `H4FD`
/// notice, which would refuse every t27 hour that lost a callback.
#[test]
fn a_cd_style_partial_batch_certifies_without_a_degraded_notice() {
    let loss = sunk_ship_history_live_lane::SunkShipProducerLoss {
        dropped_unbound: 9,
        ..Default::default()
    };
    assert!(loss.names_a_cause(), "the fixture must name a real cause");
    let payload = sunk_ship_payload(0, Some(loss));
    let decoded = sunk_ship_history_live_lane::decode_payload(&payload)
        .expect("the partial batch decodes");
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

/// d86's FAILED READS certify through the `H4FD` declaration, and the tolerance
/// entry must never become an amnesty for them.
///
/// This test was written when d86 had no tolerance entry at all, on the reading
/// that "an entry would assert that a failed read is a reviewed partial with a
/// written integrity argument. It is not: the fold withholds the domain's rows
/// entirely." That premise was true of every d86 status bit at the time,
/// because a partial history was refused wholesale downstream.
///
/// Owner ruling 2026-08-06 changed exactly one of those bits and nothing else.
/// A producer-declared partial is now an ADMITTED delivery: the fold applies its
/// rows and replay carries them, with the SHP1 loss block naming every omission.
/// That is the definition of an expected partial, so `PARTIAL_DATA` -- and only
/// `PARTIAL_DATA` -- is tolerated for this domain.
///
/// The protection the test was written for is unchanged and is what the rest of
/// it asserts: a FAILED read (`source_unavailable`, a worker panic, an
/// unreadable clock) is still non-tolerated, still withholds the rows, and must
/// still be declared in `H4FD`. The amnesty the original comment feared would be
/// a tolerance that swallowed those, and none of them is tolerated.
///
/// SCOPE. This pins the TOLERANCE BOUNDARY -- which words the domain-86 entry
/// does and does not cover, and that each uncovered one is still expressible as
/// an `H4FD` entry. It does not certify a stream, so it is not what proves the
/// certifier REQUIRES that entry; `an_undeclared_d86_payload_failure_still
/// _refuses` and `a_declared_d86_payload_failure_certifies_as_an_accounted
/// _unknown` are, and they are unchanged by this lane.
///
/// Production change that makes this fail: widening the domain-86 tolerance
/// beyond `PARTIAL_DATA`, or removing it (which would put the wholesale refusal
/// back one layer down, in the fold's failure gate).
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
                "domain 86: read_completion=2",
            )
            .is_some(),
            "{label}: the accounting path for a d86 FAILURE is the H4FD declaration"
        );
    }
    // And a failure carried ALONGSIDE the partial declaration is not laundered
    // by it: the tolerance applies only when the tolerated bit is the whole
    // failure word.
    assert_eq!(
        compact_status_failure_flags_after_expected_partial_tolerance(
            86,
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        ),
        COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        "a partial declaration does not tolerate the failure sitting beside it"
    );
}

// ---------------------------------------------------------------------
// A domain's delta baseline must survive another domain's DOMAIN_GAP
// ---------------------------------------------------------------------
//
// This is the fold-side acceptance gate for the collector's DOMAIN-SCOPED
// rejected-bracket reset. The collector used to drop EVERY domain's delta pin
// whenever ANY domain's clock bracket was rejected, so after a foreign gap an
// unrelated domain always re-keyframed. Scoping the reset to the rejected
// domain means it now emits a DELTA there instead — an interleaving the fold
// was never previously asked to decode, because the old producer could not
// produce it.
//
// Domain 29 (`ships`) is the subject because its fold takes exactly the path
// that decides the question: `fold_ships` calls `reject_delta_before_baseline`,
// so a fold that forgot d29's baseline over d64's gap would refuse hour C's
// delta and type the domain-hour unknown rather than applying it.
//
// The ships payload layout below mirrors `world_fold::fold_ships`. It is not a
// second source of truth: every byte is handed to the production decoder, and
// any drift makes the fold return an error that fails these tests loudly.

const SHIPS_DOMAIN: u16 = 29;
const SHIP_HEADER_LEN: usize = 40;
/// `world_fold::FIXED_SCALE as u32`, which `ensure_fixed_scale` requires.
const SHIP_SCALE_RAW: u32 = 100_000;
const SHIP_ROW_FLAG_DELETED: u32 = 1;
const SHIP_ROW_FLAG_DEFINITION: u32 = 1 << 2;
const SHIP_ROW_FLAG_OWNER: u32 = 1 << 3;

/// One ship the fold will hold: `(instance_id, strength_raw)`.
type Ship = (u32, u32);

/// A `ships` DOMAIN_DATA payload carrying `live` ships and tombstoning
/// `deleted` ones.
fn ships_payload(live: &[Ship], deleted: &[u32], status_flags: u32) -> Vec<u8> {
    let row_count = live.len() + deleted.len();
    let mut payload = Vec::with_capacity(SHIP_HEADER_LEN + row_count * 48);
    payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
    payload.extend_from_slice(&(row_count as u32).to_le_bytes()); // row_count
    payload.extend_from_slice(&(row_count as u32).to_le_bytes()); // changed_rows
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(live.len() as u32).to_le_bytes()); // observed_ship_rows
    payload.extend_from_slice(&0u32.to_le_bytes()); // (unread by the fold)
    payload.extend_from_slice(&(deleted.len() as u32).to_le_bytes()); // deleted_rows
    payload.extend_from_slice(&SHIP_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // reserved0
    assert_eq!(payload.len(), SHIP_HEADER_LEN, "ships header is 40 bytes");

    let mut push_row = |instance_id: u32, strength_raw: u32, definition: &str, flags: u32| {
        payload.extend_from_slice(&instance_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // task_force_id
        payload.extend_from_slice(&3u32.to_le_bytes()); // category_enum
        payload.extend_from_slice(&strength_raw.to_le_bytes());
        payload.extend_from_slice(&50_000u32.to_le_bytes()); // organisation_raw
        payload.extend_from_slice(&0u32.to_le_bytes()); // experience_raw
        payload.extend_from_slice(&0u32.to_le_bytes()); // sunk_convoys
        payload.extend_from_slice(&47u32.to_le_bytes()); // owner_tag_db_id
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
        payload.extend_from_slice(&(definition.len() as u16).to_le_bytes());
        payload.extend_from_slice(definition.as_bytes());
    };
    for (instance_id, strength_raw) in live {
        push_row(
            *instance_id,
            *strength_raw,
            "destroyer_1",
            SHIP_ROW_FLAG_OWNER | SHIP_ROW_FLAG_DEFINITION,
        );
    }
    for instance_id in deleted {
        // A tombstone carries no definition, so it must not set the flag that
        // promises one.
        push_row(*instance_id, 0, "", SHIP_ROW_FLAG_DELETED);
    }
    payload
}

/// What hour C carries for domain 29.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HourCShape {
    /// The producer RETAINED its pin across hour B's foreign gap, so hour C is
    /// a DELTA carrying only the ship whose strength changed. This is what the
    /// domain-scoped reset makes the collector emit.
    RetainedPinDelta,
    /// The producer's pin was wiped by the old blanket reset, so hour C is the
    /// whole state as a keyframe. This is what the collector emitted before.
    BlanketResetKeyframe,
    /// Seeded inequality: a delta carrying a DIFFERENT world, so a comparator
    /// that always reports equality is caught.
    DivergentDelta,
}

/// Three game-hours of a domain 29 delta chain, with domain 64's `DOMAIN_GAP`
/// optionally interrupting the middle hour.
///
/// * Hour A — a keyframe seeds d29's baseline with ship 1 at strength 500.
/// * Hour B — a delta adds ship 2, and (when `foreign_gap`) domain 64's bracket
///   is rejected and it emits an H4DG gap in the same hour.
/// * Hour C — ship 1 moves to strength 900, carried per [`HourCShape`].
///
/// Both the retained-pin and blanket-reset shapes describe the SAME world after
/// hour C: `{1 -> 900, 2 -> 700}`.
fn ships_delta_chain_stream(foreign_gap: bool, shape: HourCShape) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let mut push = |stream: &mut Vec<u8>, hour: i64, domain: u16, rtype: u16, payload: &[u8]| {
        push_record(stream, seq, raw_for(hour), domain, rtype, payload);
        seq += 1;
    };

    // Hour A — the keyframe that seeds the delta baseline.
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

    // Hour B — d29 emits a delta; with `foreign_gap`, d64 gaps in the same hour.
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

    // Hour C — the two producers part company.
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

/// **The fold-equivalence gate for the domain-scoped rejected-bracket reset.**
///
/// A domain that survived its own clock bracket keeps its delta pin across
/// another domain's rejected bracket, so it publishes a DELTA in the hour after
/// that gap where it used to publish a keyframe. This proves the consumer
/// cannot tell: the delta-carrying stream folds to a world byte-identical to
/// the keyframe-carrying one, in every hour, through the production
/// `WorldFold`.
///
/// The comparison is on the canonical per-hour snapshot hash, which is the
/// whole folded world rather than a projection of it.
///
/// What production change makes this fail: any fold behaviour that reacts to a
/// `DOMAIN_GAP` by dropping another domain's delta baseline. Hour C's delta
/// would then be refused by `reject_delta_before_baseline`, ship 1 would stay
/// at strength 500, and the two hashes would part.
#[test]
fn a_retained_delta_baseline_across_a_foreign_gap_folds_identically_to_the_keyframe() {
    let retained = fold_bytes(&ships_delta_chain_stream(true, HourCShape::RetainedPinDelta));
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

/// The same equivalence must hold with no foreign gap at all, which isolates
/// the gap as the variable rather than the delta encoding.
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
            delta.hours.last().expect("hour C is published").snapshot_hash,
            keyframe
                .hours
                .last()
                .expect("hour C is published")
                .snapshot_hash,
            "hour C must fold identically whether or not domain 64 gapped (foreign_gap = \
             {foreign_gap})"
        );
    }
    // And the foreign gap must not move the world by itself.
    let with_gap = fold_bytes(&ships_delta_chain_stream(true, HourCShape::RetainedPinDelta));
    let without_gap = fold_bytes(&ships_delta_chain_stream(
        false,
        HourCShape::RetainedPinDelta,
    ));
    for (a, b) in with_gap.hours.iter().zip(without_gap.hours.iter()) {
        assert_eq!(
            a.snapshot_hash, b.snapshot_hash,
            "hour {} changed world merely because domain 64 gapped",
            a.hour_index
        );
    }
}

/// The seeded-inequality control.
///
/// Without it, a comparator that reported equality unconditionally — or a fold
/// that ignored hour C's payload — would pass the gate above. This stream
/// differs from the retained-pin stream only in hour C's rows, and its hour C
/// must hash differently.
#[test]
fn the_ships_fold_equivalence_gate_can_tell_two_worlds_apart() {
    let world = fold_bytes(&ships_delta_chain_stream(true, HourCShape::RetainedPinDelta));
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

/// Hour C's delta really MERGES onto a retained baseline: ship 2, which hour B
/// introduced and hour C never mentions, is still in the folded world.
///
/// The proof is a tombstone. A keyframe cannot serve here: once a domain has a
/// delta baseline, `replace_or_sparse_after_first` makes EVERY subsequent d29
/// record sparse, so even a whole-state record merges and an omitted row is
/// never dropped. Absence is not deletion on this wire (`DELTA_WIRE_DESIGN.md`
/// §2.4) — only a tombstone removes a key. So the control stream tombstones
/// ship 2 explicitly, and its hour C must hash differently from the stream that
/// kept it.
///
/// What production change makes this fail: a fold that replaced rather than
/// merged on hour C's delta would lose ship 2 and land on the tombstoned
/// world's hash.
#[test]
fn the_hour_c_delta_merges_onto_the_retained_baseline() {
    let retained = fold_bytes(&ships_delta_chain_stream(true, HourCShape::RetainedPinDelta));

    // The same three hours, except hour C's delta also tombstones ship 2.
    let mut tombstoned = Vec::new();
    let mut seq = 1u64;
    let mut push = |stream: &mut Vec<u8>, hour: i64, domain: u16, rtype: u16, payload: &[u8]| {
        push_record(stream, seq, raw_for(hour), domain, rtype, payload);
        seq += 1;
    };
    for (hour, live, deleted, status) in [
        (HOUR_A, vec![(1u32, 500u32)], vec![], 0u32),
        (HOUR_B, vec![(2, 700)], vec![], COMPACT_STATUS_DELTA_DATA),
        (HOUR_C, vec![(1, 900)], vec![2u32], COMPACT_STATUS_DELTA_DATA),
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
        push(
            &mut tombstoned,
            hour,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with(1, 1, 0),
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

/// The foreign gap is genuinely present and genuinely accounted, so the gate is
/// not quietly comparing two streams with no rejected bracket in either.
#[test]
fn the_middle_hour_really_carries_the_foreign_domains_gap() {
    let with_gap = fold_bytes(&ships_delta_chain_stream(true, HourCShape::RetainedPinDelta));
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
        with_gap.hours[1].completeness.domains.contains_key(&SHIPS_DOMAIN),
        "domain 29 must still be accounted in the same hour"
    );
}

// ---------------------------------------------------------------------
// Domain 13 `tech_live` — the fold-equivalence gate for its keyed
// per-country patch
// ---------------------------------------------------------------------
//
// D13 is the one country-bundle domain that re-sent every country every hour:
// `research_slots_fingerprint` mixes the current game hour into the hash it
// uses to decide "changed", so that predicate can never say no. Measured over
// `runs/live_unlock/t26/collector_stream.bin`, all 439 countries were carried
// in all 285 records while only 2.25% of the per-country wire blocks differed
// from the block sent before them.
//
// `compact_tech_data_payload_delta` now diffs those blocks. These are the gates
// that a patch stream folds to the same world as the whole-payload stream it
// replaced — including the case the encoder must NOT treat as a removal: a
// country the reader could not sample, which drops out of the payload with no
// failure bit and which the merging fold reads as "keep what you have".
//
// The payload builder below mirrors `world_fold::fold_tech`'s layout and the
// scale words `domain_data_authority` validates. It is not a second source of
// truth: every byte goes to the production decoder, so drift makes these tests
// error rather than pass.

const TECH_DOMAIN: u16 = 13;
const TECH_HEADER_LEN: usize = 32;
/// These are `domain_data_authority`'s `COMPACT_TECH_PROGRESS_SCALE_RAW` and
/// `COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW`, and they are NOT interchangeable: a
/// record whose scale words disagree with them is excluded as malformed before
/// the fold ever sees it. Swapping the two here is how this fixture first went
/// green over two worlds that contained no research at all, which is what
/// `the_d13_fixture_actually_publishes_research_into_the_folded_world` exists
/// to stop happening again.
const TECH_PROGRESS_SCALE_RAW: u32 = 1_000;
const TECH_LIVE_PROGRESS_SCALE_RAW: u32 = 100_000;
const TECH_COUNTRY_FLAG_COMPLETED: u32 = 1;
const TECH_COUNTRY_FLAG_SLOTS: u32 = 1 << 1;

/// `(tag_db_id, completed_tech_count, research_slot_count)`.
type TechCountry = (u32, u32, u32);

/// A `tech_live` DOMAIN_DATA payload carrying `live` countries. No
/// active-research rows: the country section alone is what the delta keys on.
///
/// There is no `deleted` parameter, and that is the point — D13 has no
/// tombstone grammar, so a control that fabricated one would not be a control
/// over anything the producer can emit.
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

/// What hour C carries for domain 13.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TechHourCShape {
    /// The patch the encoder emits: only country 2 changed, and country 3 was
    /// not measured this hour so nothing is said about it.
    Patch,
    /// The whole payload the encoder emits after a reset — exactly what
    /// `compact_tech_data_payload` produces for the same read, which is the
    /// control the patch must fold identically to. Country 3 is absent for the
    /// same reason: it was not measured.
    WholePayload,
    /// The control that proves country 3 SURVIVED, rather than merely that both
    /// arms treated it the same way: it re-asserts country 3 at the value hour
    /// A gave it, alongside the other two. A stream that retained country 3
    /// folds to exactly this world; a stream that dropped it does not.
    PatchReassertingTheUnmeasuredCountry,
    /// Seeded inequality, so a comparator that always reports equality is
    /// caught.
    DivergentPatch,
}

/// Three game-hours of a domain 13 patch chain.
///
/// * Hour A — a keyframe seeds the baseline with countries 1, 2 and 3.
/// * Hour B — a patch moves country 1.
/// * Hour C — country 2 moves; country 3 is NOT measured, carried per
///   [`TechHourCShape`].
///
/// `Patch` and `WholePayload` describe the SAME world after hour C:
/// `{1 -> (11, 1), 2 -> (77, 2), 3 -> (30, 3) retained}`.
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
    // Every hour carries D18's roster, as a real capture does. Without it the
    // fold has no country identity to publish D13's research under, and the
    // gates below would compare two worlds that hold no research at all.
    //
    // The roster keeps all three countries in every hour on purpose: country 3
    // is UNMEASURED here, not departed, so
    // `evict_departed_roster_domain_fields` must not be what removes it — the
    // question these gates ask is what the ENCODER does about an unmeasured
    // country.
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

    // Hour A — the keyframe that seeds the delta baseline.
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

    // Hour B — a patch carrying only the country that changed.
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

    // Hour C — the shapes part company.
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

/// The fixture has to actually put D13's research into the folded world, or
/// every equivalence assertion below would hold vacuously over two empty
/// worlds. It did exactly that once: `domain_data_authority` excludes a D13
/// record whose two scale words disagree with the contract, and with them
/// transposed all three records were dropped as malformed while the gates went
/// green.
///
/// What production change makes this fail: anything that stops `fold_tech`
/// publishing under a resolved country key, or a change to either scale
/// constant that the fixture does not follow.
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

/// **The fold-equivalence gate for D13's keyed per-country patch.**
///
/// The patch stream and the whole-payload stream must fold to a byte-identical
/// world in every hour, through the production `WorldFold`, compared on the
/// canonical per-hour snapshot hash rather than on a projection of it.
///
/// What production change makes this fail: emitting a country the pin already
/// holds unchanged, corrupting any field a patch carries, or advancing the pin
/// to a set the payload did not carry.
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

/// An UNMEASURED country must survive, and this is what says so on the wire
/// rather than in a comment.
///
/// Country 3 is in the roster in every hour and is measured only in hour A. Its
/// research must still be in the world at hour C under both shapes. If the
/// encoder ever read "absent from this frame" as "deleted" — the tombstone this
/// domain deliberately does not have — country 3's research would be gone here
/// and this fails.
///
/// What production change makes this fail: giving
/// `compact_tech_data_payload_delta` a `deleted_row_fn` that emits a real
/// tombstone row, or dropping the carry-forward in
/// `compact_tech_delta_blocks` so the shared engine tombstones every country
/// the frame did not measure.
#[test]
fn an_unmeasured_d13_country_is_retained_by_both_shapes() {
    let patch = fold_bytes(&tech_delta_chain_stream(TechHourCShape::Patch));
    let re_asserted = fold_bytes(&tech_delta_chain_stream(
        TechHourCShape::PatchReassertingTheUnmeasuredCountry,
    ));
    // The control SPELLS OUT the world country 3 must still be in: it re-sends
    // country 3 at its hour-A value. Equality therefore says the patch arm,
    // which never mentioned country 3 after hour A, still holds it — an
    // assertion that both arms silently dropping country 3 could not satisfy.
    assert_eq!(
        patch.hours[2].snapshot_hash, re_asserted.hours[2].snapshot_hash,
        "hour C's patch dropped the unmeasured country 3: it folds to a different world than \
         the stream that re-asserts country 3 at the value hour A gave it"
    );
    // And hour C must still differ from hour B, or the comparison above would
    // be between two worlds in which nothing happened at all.
    assert_ne!(
        patch.hours[1].snapshot_hash, patch.hours[2].snapshot_hash,
        "hour C changed nothing, so this gate compares two identical worlds"
    );
}

/// The seeded-inequality control for the gate above.
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

// ---------------------------------------------------------------------------
// The exclusion ledger joined into per-hour completeness (owner ruling
// 2026-08-06). A DOMAIN_DATA record the payload authority rejects is dropped
// from the fold and recorded in the exclusion ledger, so the LOSS is accounted
// — but the hour that lost it must stop reporting that domain complete.
// ---------------------------------------------------------------------------

/// Two hours over one roster domain. Hour A's DATA payload is exact; hour B's
/// carries one trailing byte, so the payload authority rejects it, `WorldFold`
/// applies none of its rows, and the decoder writes it into the exclusion
/// ledger. Both hours carry a CLEAN `DOMAIN_PERF`, which is what made the loss
/// invisible: the collector mints its `H4FD` notice from the PERF status, so a
/// payload the authority cannot decode has no notice, no status word, and no
/// gap record — every signal a reader had for "this domain is not complete"
/// stays clean while the fold holds nothing.
///
/// `malformed_hour` selects WHICH of the two hours carries the bad payload,
/// and that choice is load-bearing rather than cosmetic. The decoder's ledger
/// is a RUNNING total: when the cursor yields the first hour, a later hour's
/// exclusion has not been read yet, so a join that ignored the hour key
/// entirely would still leave the first hour looking clean. Only putting the
/// exclusion in the EARLIER hour puts an entry in the ledger that a broken
/// join could leak forward into the later one.
fn two_hours_with_one_excluded_payload(malformed_hour: i64) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for hour in [HOUR_A, HOUR_B] {
        let exact = hour != malformed_hour;
        let mut payload = country_identity_payload(&[(47, "GER")]);
        if !exact {
            // One trailing byte the domain grammar does not consume. The record
            // stays well-FRAMED; only its payload is bad, which is exactly the
            // zero-data-loss case that degrades one domain instead of the
            // recording.
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

    // The ledger recorded the loss, against the hour the record BELONGED to.
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

    // THE BLINDNESS, STATED. Every pre-existing per-domain signal is clean for
    // hour B, so none of them can be the thing that says the domain is
    // incomplete. These assertions hold both before and after the ledger join —
    // they are the reason a NEW fact was needed rather than a re-reading of an
    // old one.
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

    // THE RULING. The hour's completeness report must say the domain is not
    // complete, and must name the ledger's own reason for it.
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

/// The exclusion is attributed to the hour the record BELONGED to, never to the
/// hour it happened to be processed in (owner stop condition: hour
/// misattribution is unacceptable).
///
/// The offending record is in the EARLIER hour on purpose. By the time the
/// cursor yields hour B, hour A's exclusion is already sitting in the decoder's
/// running ledger, so a join that matched on anything other than the entry's
/// own `game_hour_raw` — or that simply took every ledger entry it was handed —
/// would charge hour B for a loss it never suffered. Putting the bad record in
/// the LATER hour cannot test this: the ledger is still empty when the earlier
/// hour is reported, so the earlier hour looks clean either way.
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

    // Hour A owns the loss.
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

    // Hour B decoded and folded its own payload, and must not inherit hour A's.
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

    // And the two hours are therefore NOT the same hour to `compare`, which
    // decides equivalence on the completeness record.
    assert_ne!(
        run.hours[0].completeness, run.hours[1].completeness,
        "an hour that lost a payload and an hour that did not must not compare equal"
    );
}

/// An exclusion inside a REFUSED hour has no completeness record to reach, and
/// that is not a hole in the accounting: the hour published nothing at all, and
/// the refusal already says so. The attribution check must accept it rather
/// than demand it be charged to some published hour — charging it to the next
/// published hour is precisely the misattribution the ruling forbids.
#[test]
fn an_exclusion_in_a_refused_hour_is_accounted_by_the_refusal_not_the_next_hour() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;

    // Hour A: a malformed payload, then the hour is refused outright.
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

    // Hour B publishes normally.
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

/// The resume-position walk has to do two different things inside ONE call: step
/// OVER an exclusion belonging to an earlier hour that closed without
/// publishing, and then attribute an exclusion belonging to the hour being
/// reported. Both ledger entries are already present when the single published
/// hour is processed, so this pins the skip and the attribution against each
/// other — a walk that stopped at the first non-matching entry would attribute
/// nothing, and one that took every entry it stepped over would charge hour B
/// with hour A's loss as well.
#[test]
fn the_ledger_walk_skips_a_refused_hours_exclusion_and_still_takes_the_current_ones() {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    let mut malformed = country_identity_payload(&[(47, "GER")]);
    malformed.push(0xAA);

    // Hour A: an excluded payload, then refused.
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

    // Hour B: an excluded payload too, and it PUBLISHES.
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
    assert_eq!(reported["state"], serde_json::json!("excluded"), "{report:#}");
}

