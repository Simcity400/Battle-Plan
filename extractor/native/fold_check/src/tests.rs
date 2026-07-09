//! In-memory unit tests for the fold-equivalence gate. They build a small
//! synthetic H4CR stream (via the same wire layout the collector emits), fold it
//! through the production `WorldFold`, and assert stable hashing, PERF-record
//! ignoring, mutation detection, and truncated-tail handling.

use super::*;
use compact_contract::calendar::DATE_EPOCH_CONST;
use std::io::Cursor;

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
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
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

/// A DOMAIN_PERF payload (>= 40 bytes) that the fold must ignore entirely.
fn perf_payload() -> Vec<u8> {
    vec![0u8; 48]
}

const HOUR_A: i64 = 16_959_360;
const HOUR_B: i64 = 16_959_361;
const HOUR_C: i64 = 16_959_362;

/// Three complete game-hours. Hour B carries a PERF record (must be ignored) and
/// hour C a second country. Returns the stream and the byte offset of the first
/// `tag_db_id` in hour A's domain-18 payload (a real payload byte, for mutation).
fn synthetic_stream() -> (Vec<u8>, usize) {
    let mut s = Vec::new();
    let mut seq = 1u64;

    // Hour A: one country, then frame_end.
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
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &[],
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
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &[],
    );
    seq += 1;

    // Hour C: two countries.
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
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &[],
    );

    (s, mutation_offset)
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
fn perf_record_is_ignored_by_the_fold() {
    // A stream identical to the synthetic one but WITHOUT hour B's PERF record
    // must fold to the identical per-hour hashes: the PERF record carries no
    // folded state.
    let (with_perf, _) = synthetic_stream();
    let run_with = fold_bytes(&with_perf);
    assert_eq!(run_with.totals.perf_records, 1, "perf record was seen");

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
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &[],
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
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &[],
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
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &[],
    );
    let run_without = fold_bytes(&s);

    assert_eq!(run_without.totals.perf_records, 0);
    assert_eq!(
        chain_hash(&run_with.hours),
        chain_hash(&run_without.hours),
        "the PERF record must not change any folded snapshot"
    );
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
fn truncated_tail_reports_complete_hours_only() {
    let (stream, _) = synthetic_stream();
    // Chop the last 5 bytes → the final frame_end record is incomplete.
    let truncated = &stream[..stream.len() - 5];
    let run = collect_run(Cursor::new(truncated.to_vec()), truncated.len() as u64)
        .expect("fold truncated stream");
    // Hours A and B closed; hour C's frame_end never completed.
    assert_eq!(run.hours.len(), 2, "only complete hours are emitted");
    assert_eq!(run.hours[0].hour_index, HOUR_A);
    assert_eq!(run.hours[1].hour_index, HOUR_B);
    assert!(
        run.totals.truncated_tail_bytes > 0,
        "incomplete tail is reported"
    );
}
