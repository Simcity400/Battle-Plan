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
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION, COMPACT_STATUS_PARTIAL_DATA,
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
    assert!(!production.domains.contains_key(&18));
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
fn malformed_domain_payload_is_rejected_even_at_a_closed_hour_boundary() {
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
    let err = match collect_run(Cursor::new(stream.clone()), stream.len() as u64) {
        Ok(_) => panic!("fold_check must reject malformed DOMAIN_DATA"),
        Err(err) => err,
    };
    assert!(err.contains("failed exact validation"), "{err}");
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
        err.contains("final record must be canonical FRAME_END"),
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
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../snapshot/tests/data/legacy/pre222_store_deflate.battleplan");
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
