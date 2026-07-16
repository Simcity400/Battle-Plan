//! Tests for the shared decoder facade. They prove:
//!   * per-hour yielded bytes + hashes equal the reference batch fold (the same
//!     canonical snapshot bytes `fold_check` hashes);
//!   * both source kinds open (a synthetic H4CR stream and the checked-in legacy
//!     `.battleplan` container);
//!   * iteration is bounded (no previous hour retained after a yield);
//!   * `seek_to_hour` reproduces linear iteration exactly.

use super::*;
use compact_contract::calendar::DATE_EPOCH_CONST;
use compact_contract::{
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
};
use recorder_host::world_fold::WorldFold;
use std::io::{Cursor, Read};
use std::path::PathBuf;

// --- synthetic H4CR stream builders (same wire layout the collector emits) ---

fn raw_for(hour_index: i64) -> i32 {
    DATE_EPOCH_CONST + hour_index as i32
}

fn push_record(
    out: &mut Vec<u8>,
    seq: u64,
    game_hour_raw: i32,
    domain: u16,
    ty: u16,
    payload: &[u8],
) {
    push_record_with_flags(out, seq, game_hour_raw, domain, ty, 0, payload);
}

fn push_record_with_flags(
    out: &mut Vec<u8>,
    seq: u64,
    game_hour_raw: i32,
    domain: u16,
    ty: u16,
    flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(payload);
}

/// domain-18 country_identity payload.
fn country_identity_payload(rows: &[(u32, &str)]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    for (id, tag) in rows {
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        p.extend_from_slice(tag.as_bytes());
        p.push(1); // cosmetic null
    }
    p
}

fn frame_end_payload() -> Vec<u8> {
    vec![0u8; 32]
}

/// FRAME_END seal for an hour that emitted `non_probe_perf_domains` distinct
/// non-probe DOMAIN_PERF domains. The production collector writes one
/// DOMAIN_PERF record per emitted domain and stamps that same count into both
/// the emitted- and scheduled-domain seal fields (collector::lib.rs
/// `append_compact_frame_end_record`, called with `emitted_domains,
/// emitted_domains`), so the strict boundary check cross-references these
/// fields against the unique non-probe DOMAIN_PERF domains observed in the
/// hour. A frame with no PERF records still uses the all-zero `frame_end_payload`.
fn frame_end_payload_with_perf_domains(non_probe_perf_domains: u32) -> Vec<u8> {
    let mut p = vec![0u8; 32];
    p[16..20].copy_from_slice(&non_probe_perf_domains.to_le_bytes());
    p[20..24].copy_from_slice(&non_probe_perf_domains.to_le_bytes());
    p
}

fn perf_payload_with_trailer(name: &str) -> Vec<u8> {
    let mut payload = vec![0u8; 40];
    payload.extend_from_slice(b"PDU1");
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&7u64.to_le_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload
}

const DOMAIN_FRAME: u16 = 1000;

const HOUR_A: i64 = 16_959_360;
const HOUR_B: i64 = 16_959_361;
const HOUR_C: i64 = 16_959_362;

/// Three game-hours, each a country_identity frame + a FRAME_END.
fn three_hour_stream() -> Vec<u8> {
    let mut s = Vec::new();
    let mut seq = 1u64;
    for (h, rows) in [
        (HOUR_A, vec![(47u32, "GER")]),
        (HOUR_B, vec![(47, "GER")]),
        (HOUR_C, vec![(47, "GER"), (48, "FRA")]),
    ] {
        push_record(
            &mut s,
            seq,
            raw_for(h),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&rows),
        );
        seq += 1;
        push_record(
            &mut s,
            seq,
            raw_for(h),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        seq += 1;
    }
    s
}

/// The reference: the batch in-memory production fold (what `fold_check` hashes).
fn reference_fold(stream: &[u8], game_id: &str) -> Vec<(i64, String, Vec<u8>)> {
    let mut fold = WorldFold::new(game_id);
    fold.ingest_stream(stream);
    fold.finish()
}

// --- temp dir helper ---

struct TmpDir(PathBuf);
impl TmpDir {
    fn new(tag: &str) -> TmpDir {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "replay_decoder_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn legacy_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../snapshot/tests/data/legacy/pre222_store_deflate.battleplan")
}

// ---------------------------------------------------------------------------

/// A synthetic stream, folded through the cursor, yields per-hour bytes and
/// hashes byte-identical to the reference batch fold (== `fold_check` hashes).
#[test]
fn cursor_hours_match_reference_batch_fold() {
    let stream = three_hour_stream();
    let reference = reference_fold(&stream, "fold_check");
    assert_eq!(reference.len(), 3);

    let mut cursor = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );
    assert!(
        cursor.stats().certification_verdict().is_err(),
        "an unconsumed cursor cannot certify before EOF"
    );
    assert_eq!(cursor.metadata().game_id, "fold_check");

    let mut got = Vec::new();
    while let Some(h) = cursor.next_hour() {
        got.push(h.expect("no io error"));
    }
    assert_eq!(got.len(), 3);
    for (hour, (rh, rd, rb)) in got.iter().zip(reference.iter()) {
        assert_eq!(hour.hour_index, *rh);
        assert_eq!(&hour.date, rd);
        assert_eq!(&hour.snapshot, rb, "hour {} snapshot bytes differ", rh);
        assert_eq!(
            hour.snapshot_sha256(),
            snapshot::sha256::Sha256::hex(rb),
            "hour {} hash differs from reference",
            rh
        );
    }

    let stats = cursor.stats();
    assert_eq!(stats.hours_emitted, 3);
    assert_eq!(stats.frame_end_records, 3);
    assert_eq!(stats.data_records, 3);
    assert_eq!(stats.truncated_tail_bytes, 0);
    assert!(stats.stop_reason.is_none());
    assert!(stats.ends_at_frame_end);
    stats
        .certification_verdict()
        .expect("clean closed stream must certify");
    // domain 18 fed each hour.
    assert_eq!(stats.per_domain_data_records.get(&18), Some(&3));
    // per-hour counts land on the hours.
    assert_eq!(got[0].domain_data_counts.get(&18), Some(&1));
}

/// Bounded memory: at no point does the cursor retain a previous hour. After
/// each yield the ready buffer is empty (one hour closes per FRAME_END here).
#[test]
fn iteration_is_bounded_no_previous_hour_retained() {
    let stream = three_hour_stream();
    let mut cursor = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );

    let mut count = 0;
    while let Some(h) = cursor.next_hour() {
        let _ = h.expect("no io error");
        count += 1;
        // The just-yielded hour is gone and no other is held.
        assert_eq!(
            cursor.buffered_hours(),
            0,
            "cursor retained {} hours after yielding hour {}",
            cursor.buffered_hours(),
            count
        );
    }
    assert_eq!(count, 3);
}

/// A truncated stream (chopped mid-record) yields only complete hours and
/// reports the truncated tail — no panic, no partial hour.
#[test]
fn truncated_tail_yields_complete_hours_only() {
    let mut stream = three_hour_stream();
    stream.truncate(stream.len() - 5); // chop hour C's frame_end
    let total = stream.len() as u64;
    let mut cursor = open_reader(Cursor::new(stream), total, "fold_check");
    let hours: Vec<_> = cursor.by_ref().map(|h| h.unwrap().hour_index).collect();
    assert_eq!(hours, vec![HOUR_A, HOUR_B]);
    assert!(cursor.stats().truncated_tail_bytes > 0);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("prefix recovery must never certify a torn tail");
    assert!(err.contains("truncated/corrupt tail"), "{err}");
    assert!(err.contains("stream boundary is not closed"), "{err}");
}

#[test]
fn malformed_domain_payload_degrades_for_diagnostics_but_cannot_certify() {
    let mut stream = Vec::new();
    let mut malformed = country_identity_payload(&[(47, "GER")]);
    malformed.push(0xAA); // exact decoder must reject unconsumed residue
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
        &frame_end_payload(),
    );

    let mut cursor = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );
    let hours = cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect("malformed domain payload degrades without aborting diagnostics");
    assert_eq!(hours.len(), 1);
    assert_eq!(cursor.stats().malformed_payload_records, 1);
    assert!(cursor.stats().ends_at_frame_end);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("malformed payload must fail certification");
    assert!(err.contains("failed exact validation"), "{err}");
}

#[test]
fn open_hour_at_eof_is_not_truncation_but_cannot_certify() {
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    let mut cursor = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );
    assert!(cursor.next_hour().is_none());
    assert_eq!(cursor.stats().truncated_tail_bytes, 0);
    assert!(cursor.stats().stop_reason.is_none());
    assert!(!cursor.stats().ends_at_frame_end);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("open hour at EOF must fail certification");
    assert!(err.contains("left open at EOF"), "{err}");
}

#[test]
fn sequence_gap_with_closed_hour_cannot_certify() {
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
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut cursor = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );
    assert_eq!(cursor.by_ref().count(), 1);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("sequence discontinuity must fail certification");
    assert!(err.contains("sequence discontinuity"), "{err}");
}

#[test]
fn reserved_or_misplaced_record_flags_are_rejected_before_data_decode() {
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
        &frame_end_payload(),
    );
    let mut cursor = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );
    cursor.by_ref().for_each(|result| {
        result.expect("record-envelope failure remains diagnostic-readable");
    });
    assert_eq!(
        cursor.stats().malformed_payload_records,
        0,
        "envelope rejection is distinct from exact payload-decoder rejection"
    );
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("misplaced PERF flag on DATA must fail certification");
    assert!(err.contains("rejected bits"), "{err}");
}

#[test]
fn perf_payload_and_announced_trailer_are_exact() {
    let mut valid = Vec::new();
    push_record(
        &mut valid,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record_with_flags(
        &mut valid,
        2,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
        &perf_payload_with_trailer("read_us"),
    );
    push_record(
        &mut valid,
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        // One non-probe DOMAIN_PERF domain (domain 18) was emitted this hour, so
        // the production writer seals emitted=scheduled=1; the all-zero payload
        // would misrepresent the frame as PERF-free and fail the boundary check.
        &frame_end_payload_with_perf_domains(1),
    );
    let mut cursor = open_reader(Cursor::new(valid.clone()), valid.len() as u64, "fold_check");
    assert_eq!(cursor.by_ref().count(), 1);
    cursor
        .stats()
        .certification_verdict()
        .expect("well-formed announced PERF trailer must certify");

    let mut unannounced = Vec::new();
    push_record(
        &mut unannounced,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut unannounced,
        2,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &vec![0u8; 48],
    );
    push_record(
        &mut unannounced,
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut cursor = open_reader(
        Cursor::new(unannounced.clone()),
        unannounced.len() as u64,
        "fold_check",
    );
    assert_eq!(cursor.by_ref().count(), 1);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("unannounced PERF trailer must fail certification");
    assert!(err.contains("unannounced trailer"), "{err}");
}

#[test]
fn frame_end_envelope_and_open_hour_are_exact() {
    for (domain, payload, flags, expected) in [
        (41, vec![0u8; 32], 0, "FRAME_END domain"),
        (DOMAIN_FRAME, vec![0u8; 31], 0, "FRAME_END payload length"),
        (
            DOMAIN_FRAME,
            vec![0u8; 32],
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            "rejected bits",
        ),
    ] {
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record_with_flags(
            &mut stream,
            2,
            raw_for(HOUR_A),
            domain,
            COMPACT_RECORD_TYPE_FRAME_END,
            flags,
            &payload,
        );
        let mut cursor = open_reader(
            Cursor::new(stream.clone()),
            stream.len() as u64,
            "fold_check",
        );
        cursor.by_ref().for_each(|result| {
            result.expect("invalid frame envelope remains diagnostic-readable");
        });
        let err = cursor
            .stats()
            .certification_verdict()
            .expect_err("invalid FRAME_END envelope must fail certification");
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }

    let mut bare = Vec::new();
    push_record(
        &mut bare,
        1,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut cursor = open_reader(Cursor::new(bare.clone()), bare.len() as u64, "fold_check");
    cursor.by_ref().for_each(|result| {
        result.expect("bare FRAME_END remains diagnostic-readable");
    });
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("bare FRAME_END must fail certification");
    assert!(err.contains("without an open DOMAIN_DATA hour"), "{err}");
}

#[test]
fn sequence_starts_at_one_never_wraps_and_stream_ends_on_frame_end() {
    assert!(strict_sequence_error(None, 2, 0)
        .expect("wrong first sequence must fail")
        .contains("expected 1"));
    assert!(strict_sequence_error(Some(u64::MAX), 0, 32)
        .expect("u64 sequence wrap must fail")
        .contains("sequence overflow"));

    let mut wrapping = Vec::new();
    push_record(
        &mut wrapping,
        u64::MAX,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut wrapping,
        0,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut cursor = open_reader(
        Cursor::new(wrapping.clone()),
        wrapping.len() as u64,
        "fold_check",
    );
    assert_eq!(cursor.by_ref().count(), 1);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("wrapping sequence must fail certification");
    assert!(err.contains("starts at"), "{err}");

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
        &frame_end_payload(),
    );
    push_record(
        &mut trailing_perf,
        3,
        raw_for(HOUR_A + 1),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &vec![0u8; 40],
    );
    let mut cursor = open_reader(
        Cursor::new(trailing_perf.clone()),
        trailing_perf.len() as u64,
        "fold_check",
    );
    assert_eq!(cursor.by_ref().count(), 1);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("trailing PERF must fail final-boundary certification");
    assert!(
        err.contains("final record must be canonical FRAME_END"),
        "{err}"
    );
}

/// The checked-in legacy `.battleplan` opens as a container source, and its one
/// folded hour matches a reference fold of the same verified stream bytes.
#[test]
fn legacy_container_opens_and_matches_reference() {
    let path = legacy_fixture_path();

    // Auto-detect must classify it as a container.
    match Source::detect(&path).expect("detect legacy fixture") {
        Source::Container(_) => {}
        Source::StreamFile(_) => panic!("legacy fixture misdetected as a raw stream"),
    }

    let mut cursor = open(&path).expect("open legacy container");
    assert_eq!(cursor.metadata().game_id, "legacy");
    assert_eq!(cursor.metadata().schema_version, "1");
    assert!(!cursor.metadata().domains.is_empty());

    let hours: Vec<WorldHour> = cursor.by_ref().map(|h| h.expect("no io error")).collect();
    assert_eq!(hours.len(), 1, "legacy fixture has exactly one hour");
    assert_eq!(hours[0].hour_index, 16_959_360);
    // Snapshot parses as JSON and carries the expected header fields.
    let v = hours[0].to_json().expect("snapshot is valid JSON");
    assert_eq!(
        v.get("hour_index").and_then(|x| x.as_i64()),
        Some(16_959_360)
    );
    assert_eq!(v.get("game_id").and_then(|x| x.as_str()), Some("legacy"));

    // Reference: read the verified stream bytes straight from the container and
    // batch-fold them under the same game id.
    let container = snapshot::container::Container::open(
        &path,
        &snapshot::container::ExpectedIdentity::default(),
    )
    .expect("open container directly");
    let entry = container.entries_by_role(STREAM_ROLE);
    assert_eq!(entry.len(), 1);
    let stream_bytes = container
        .read_entry_bytes(&entry[0].path)
        .expect("read stream entry");
    let reference = reference_fold(&stream_bytes, "legacy");
    assert_eq!(reference.len(), 1);
    assert_eq!(
        hours[0].snapshot, reference[0].2,
        "legacy hour bytes differ"
    );
}

/// `seek_to_hour` reproduces linear iteration byte-for-byte, from a file source.
#[test]
fn zstd_stream_round_trip_and_fold_identity() {
    let dir = TmpDir::new("zstd_fold_identity");
    let stream = three_hour_stream();
    let raw_path = dir.0.join("collector_stream.raw.bin");
    let zstd_path = dir.0.join("collector_stream.bin");
    let decoded_path = dir.0.join("collector_stream.decoded.bin");
    std::fs::write(&raw_path, &stream).unwrap();
    compact_stream::encode_file(
        &raw_path,
        &zstd_path,
        compact_stream::StreamCompression::Zstd,
    )
    .unwrap();
    compact_stream::decode_file(&zstd_path, &decoded_path).unwrap();
    assert_eq!(
        std::fs::read(&decoded_path).unwrap(),
        stream,
        "zstd round-trip changed canonical H4CR bytes"
    );

    let opts = OpenOptions {
        game_id: Some("fold_check".to_string()),
    };
    let mut raw_cursor = Source::detect(&raw_path)
        .unwrap()
        .open_cursor(&opts)
        .unwrap();
    let raw_hours: Vec<WorldHour> = raw_cursor.by_ref().map(|hour| hour.unwrap()).collect();
    let raw_stats = raw_cursor.stats().clone();

    let mut zstd_cursor = Source::detect(&zstd_path)
        .unwrap()
        .open_cursor(&opts)
        .unwrap();
    let zstd_hours: Vec<WorldHour> = zstd_cursor.by_ref().map(|hour| hour.unwrap()).collect();
    let zstd_stats = zstd_cursor.stats().clone();

    assert_eq!(zstd_hours, raw_hours, "zstd input changed folded hours");
    assert_eq!(raw_stats.stream_bytes, stream.len() as u64);
    assert_eq!(zstd_stats.stream_bytes, stream.len() as u64);
    assert_eq!(zstd_stats.bytes_folded, raw_stats.bytes_folded);
    assert_eq!(zstd_stats.records_total, raw_stats.records_total);
    assert_eq!(zstd_stats.hours_emitted, raw_stats.hours_emitted);
    assert_eq!(zstd_stats.stop_reason, None);
}

#[test]
fn torn_zstd_frame_at_complete_h4cr_boundary_is_a_decode_error() {
    let dir = TmpDir::new("torn_zstd_boundary");
    let stream = three_hour_stream();
    let raw_path = dir.0.join("collector_stream.raw.bin");
    let zstd_path = dir.0.join("collector_stream.bin");
    std::fs::write(&raw_path, &stream).unwrap();
    compact_stream::encode_file(
        &raw_path,
        &zstd_path,
        compact_stream::StreamCompression::Zstd,
    )
    .unwrap();

    let mut stored = std::fs::read(&zstd_path).unwrap();
    stored.pop().expect("checksummed zstd frame is non-empty");
    std::fs::write(&zstd_path, stored).unwrap();

    let mut decoded_reader = compact_stream::open(&zstd_path).unwrap();
    let mut decoded = Vec::new();
    let err = decoded_reader
        .read_to_end(&mut decoded)
        .expect_err("torn frame must fail after yielding its decoded prefix");
    assert_eq!(
        decoded, stream,
        "the physical truncation lands exactly after the final FRAME_END record"
    );
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);

    let opts = OpenOptions {
        game_id: Some("fold_check".to_string()),
    };
    let mut cursor = Source::detect(&zstd_path)
        .unwrap()
        .open_cursor(&opts)
        .unwrap();
    let err = cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect_err("replay decoder must not accept complete records from a torn frame");
    assert!(err.to_string().contains("incomplete zstd frame"));
}

// Fix round 2: a byte flip that corrupts a record HEADER makes framing stop
// gracefully BEFORE the zstd footer — the checksum would never run. On a
// compressed source the cursor must hard-error (draining to EOF to force the
// checksum first); on a raw source the tolerant stop is preserved exactly.
#[test]
fn header_corruption_in_compressed_stream_is_a_hard_decode_error() {
    let dir = TmpDir::new("zstd_header_flip");
    let clean = three_hour_stream();
    let mut corrupted = clean.clone();
    let first_len = compact_contract::parse_compact_record_header(&clean)
        .unwrap()
        .record_len();
    corrupted[first_len] ^= 0xFF; // clobber the second record's magic byte

    let raw_path = dir.0.join("collector_stream.raw.bin");
    let zstd_src = dir.0.join("src.bin");
    let zstd_path = dir.0.join("collector_stream.bin");
    std::fs::write(&raw_path, &corrupted).unwrap();
    std::fs::write(&zstd_src, &corrupted).unwrap();
    compact_stream::encode_file(
        &zstd_src,
        &zstd_path,
        compact_stream::StreamCompression::Zstd,
    )
    .unwrap();

    // Compressed (this path is also exactly what fold_check drives): hard error.
    let opts = OpenOptions {
        game_id: Some("fold_check".to_string()),
    };
    let mut cursor = Source::detect(&zstd_path)
        .unwrap()
        .open_cursor(&opts)
        .unwrap();
    let err = cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect_err("compressed header corruption must not end as a clean short stream");
    assert!(
        err.to_string()
            .contains("framing stopped before physical EOF"),
        "unexpected error: {err}"
    );

    // Raw: tolerant stop-with-reason behavior byte-identical to before.
    let mut raw_cursor = Source::detect(&raw_path)
        .unwrap()
        .open_cursor(&opts)
        .unwrap();
    let hours = raw_cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect("raw torn/corrupt tails keep the tolerant stop");
    assert!(
        hours.is_empty(),
        "corruption before the first FRAME_END emits no hours"
    );
    assert!(raw_cursor.stats().stop_reason.is_some());
    assert!(
        raw_cursor.stats().certification_verdict().is_err(),
        "diagnostic raw-prefix tolerance must not become certification"
    );
}

/// Scripted reader: yields `prefix` once, then the scripted tail behavior —
/// either a hard error (simulating the strict zstd reader hitting a checksum
/// mismatch/torn footer DURING the post-framing-stop drain) or clean EOF
/// (storage intact ⇒ the framing stop itself must be the hard error). These
/// discriminate the drain: deleting it would leave the checksum error
/// unobserved and the immediate framing error indistinguishable.
struct ScriptedReader {
    prefix: Vec<u8>,
    served: bool,
    tail_error: Option<(io::ErrorKind, &'static str)>,
}

impl Read for ScriptedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.served {
            self.served = true;
            let n = self.prefix.len().min(buf.len());
            buf[..n].copy_from_slice(&self.prefix[..n]);
            assert_eq!(n, self.prefix.len(), "test prefix must fit one chunk");
            return Ok(n);
        }
        match self.tail_error {
            Some((kind, msg)) => Err(io::Error::new(kind, msg)),
            None => Ok(0),
        }
    }
}

fn one_record_then_garbage() -> Vec<u8> {
    let clean = three_hour_stream();
    let first_len = compact_contract::parse_compact_record_header(&clean)
        .unwrap()
        .record_len();
    let mut bytes = clean[..first_len].to_vec();
    bytes.extend_from_slice(b"XXXXGARBAGEHEADERXXXXGARBAGEHEADER");
    bytes
}

// The ROOT error found during the drain must propagate — not be replaced by
// the framing error (and not be skipped, which deleting the drain would do).
#[test]
fn drain_after_framing_stop_propagates_the_storage_root_error() {
    let reader = ScriptedReader {
        prefix: one_record_then_garbage(),
        served: false,
        tail_error: Some((
            io::ErrorKind::InvalidData,
            "simulated zstd checksum mismatch",
        )),
    };
    let mut cursor =
        HourCursor::from_reader_compressed(Box::new(reader), 0, "gid".to_string(), true);
    let err = cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect_err("checksum error during drain must surface");
    assert!(
        err.to_string().contains("simulated zstd checksum mismatch"),
        "root error must propagate, got: {err}"
    );
}

// Intact storage after a framing stop on a compressed source: the stop itself
// is the hard error.
#[test]
fn framing_stop_with_intact_storage_is_itself_the_hard_error() {
    let reader = ScriptedReader {
        prefix: one_record_then_garbage(),
        served: false,
        tail_error: None,
    };
    let mut cursor =
        HourCursor::from_reader_compressed(Box::new(reader), 0, "gid".to_string(), true);
    let err = cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect_err("compressed framing stop must not end cleanly");
    assert!(
        err.to_string()
            .contains("framing stopped before physical EOF"),
        "unexpected error: {err}"
    );
}

#[test]
fn seek_matches_linear_iteration() {
    let dir = TmpDir::new("seek");
    let stream = three_hour_stream();
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, &stream).unwrap();

    let source = Source::StreamFile(path.clone());
    let opts = OpenOptions {
        game_id: Some("fold_check".to_string()),
    };

    // Linear reference.
    let mut cursor = source.open_cursor(&opts).unwrap();
    let linear: Vec<WorldHour> = cursor.by_ref().map(|h| h.unwrap()).collect();
    assert_eq!(linear.len(), 3);

    // Index built on the same pass; the first hour is the one cheap seek point.
    let index = cursor
        .take_index()
        .expect("complete source-backed cursor must mint a certified index");
    assert_eq!(index.hour_range(), Some((HOUR_A, HOUR_C)));
    assert_eq!(index.cheap_seek_points(), vec![HOUR_A]);

    // A standalone build_index pass agrees.
    let index2 = build_index(&source, &opts).unwrap();
    assert_eq!(index, index2);

    // Index survives a JSON round-trip.
    let round = StreamIndex::from_json(&index.to_json()).expect("index json round-trip");
    assert_eq!(round, index);

    for want in [HOUR_A, HOUR_B, HOUR_C] {
        let seeked = seek_to_hour(&source, &index, want, &opts).expect("seek");
        let expected = linear.iter().find(|h| h.hour_index == want).unwrap();
        assert_eq!(&seeked, expected, "seek to hour {want} != linear");
    }

    // A missing hour is a clean error, not a panic.
    match seek_to_hour(&source, &index, 999_999, &opts) {
        Err(SeekError::HourNotFound(999_999)) => {}
        other => panic!("expected HourNotFound, got {other:?}"),
    }
}

fn assert_invalid_index(result: Result<WorldHour, SeekError>) {
    match result {
        Err(SeekError::InvalidIndex(_)) => {}
        other => panic!("expected InvalidIndex, got {other:?}"),
    }
}

#[test]
fn authoritative_index_requires_eof_certification_and_a_stable_source() {
    let stream = three_hour_stream();
    let mut early = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );
    early.next_hour().unwrap().unwrap();
    assert_eq!(
        early.take_index(),
        Err(IndexCertificationError::NotExhausted),
        "a valid prefix must not mint an authoritative index"
    );

    let mut anonymous = open_reader(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
    );
    anonymous.by_ref().for_each(|hour| {
        hour.expect("reader stream is valid");
    });
    assert_eq!(
        anonymous.take_index(),
        Err(IndexCertificationError::UnidentifiedSource),
        "reader-only decode is diagnostic/non-persistable without source identity"
    );

    let dir = TmpDir::new("malformed_suffix_index");
    let path = dir.0.join("collector_stream.bin");
    let mut malformed_suffix = stream;
    malformed_suffix.extend_from_slice(b"H4C");
    std::fs::write(&path, malformed_suffix).unwrap();
    match build_index(
        &Source::StreamFile(path),
        &OpenOptions {
            game_id: Some("fold_check".to_string()),
        },
    ) {
        Err(OpenError::Certification(message)) => {
            assert!(message.contains("not certified"), "{message}");
        }
        other => panic!("malformed suffix must prevent authoritative index: {other:?}"),
    }
}

#[test]
fn seek_authenticates_offsets_flags_and_persisted_source_binding() {
    let dir = TmpDir::new("index_authentication");
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, three_hour_stream()).unwrap();
    let source = Source::StreamFile(path);
    let opts = OpenOptions {
        game_id: Some("fold_check".to_string()),
    };
    let index = build_index(&source, &opts).unwrap();

    let mut forged_offset = index.clone();
    forged_offset.hours[1].start_offset += 1;
    assert_invalid_index(seek_to_hour(&source, &forged_offset, HOUR_B, &opts));

    let mut forged_flag = index.clone();
    forged_flag.hours[1].cheap_seek_point = true;
    assert_invalid_index(seek_to_hour(&source, &forged_flag, HOUR_B, &opts));

    let mut forged_initial_offset = index.clone();
    forged_initial_offset.hours[0].start_offset = 1;
    assert_invalid_index(seek_to_hour(&source, &forged_initial_offset, HOUR_A, &opts));

    for forged in [
        {
            let mut value = index.clone();
            value.source.decoded_sha256 = "0".repeat(64);
            value
        },
        {
            let mut value = index.clone();
            value.source.decoded_len += 1;
            value
        },
        {
            let mut value = index.clone();
            value.source.canonical_path.push_str(".wrong-source");
            value
        },
    ] {
        let persisted = StreamIndex::from_json(&forged.to_json())
            .expect("well-shaped forged binding survives syntax parsing");
        assert_invalid_index(seek_to_hour(&source, &persisted, HOUR_A, &opts));
    }
}

#[test]
fn seek_rejects_wrong_source_and_valid_prefix_copy_mutations() {
    let dir = TmpDir::new("index_source_mutation");
    let original_path = dir.0.join("original.bin");
    let copy_path = dir.0.join("copy.bin");
    let stream = three_hour_stream();
    std::fs::write(&original_path, &stream).unwrap();
    std::fs::write(&copy_path, &stream).unwrap();
    let original = Source::StreamFile(original_path.clone());
    let copied = Source::StreamFile(copy_path);
    let opts = OpenOptions {
        game_id: Some("fold_check".to_string()),
    };
    let index = build_index(&original, &opts).unwrap();

    assert_invalid_index(seek_to_hour(&copied, &index, HOUR_A, &opts));

    let mut malformed_suffix = stream.clone();
    malformed_suffix.extend_from_slice(b"H4C");
    std::fs::write(&original_path, malformed_suffix).unwrap();
    match seek_to_hour(&original, &index, HOUR_A, &opts) {
        Err(SeekError::Open(OpenError::Certification(_))) => {}
        other => panic!("malformed suffix must prevent seek result: {other:?}"),
    }

    // Preserve the complete prefix through the first two hours, but change a
    // valid same-length payload in the final hour. An early-return seek to the
    // unchanged first hour must still reject the altered suffix identity.
    let mut prefix_copy_mutation = stream;
    let fra = prefix_copy_mutation
        .windows(3)
        .rposition(|window| window == b"FRA")
        .expect("third-hour FRA row exists");
    prefix_copy_mutation[fra..fra + 3].copy_from_slice(b"ITA");
    std::fs::write(&original_path, prefix_copy_mutation).unwrap();
    assert_invalid_index(seek_to_hour(&original, &index, HOUR_A, &opts));
}

/// Metadata is available without folding: schema version + the compact domain
/// registry (id ↔ name).
#[test]
fn metadata_exposes_schema_and_domain_registry() {
    let stream = three_hour_stream();
    let cursor = open_reader(Cursor::new(stream.clone()), stream.len() as u64, "replay");
    let meta = cursor.metadata();
    assert_eq!(meta.schema_version, "1");
    assert!(
        meta.domains.len() >= 10,
        "domain registry should be populated"
    );
    // Domain 53 is diplomacy_intel_operatives_live (a known production domain).
    assert!(meta.domain_name(53).is_some());
    // Every listed domain has a non-empty wire name and a nonzero id.
    for d in &meta.domains {
        assert_ne!(d.id, 0);
        assert!(!d.wire_name.is_empty());
    }
}
