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
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA,
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
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
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
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[],
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
    let zstd_hours: Vec<WorldHour> =
        zstd_cursor.by_ref().map(|hour| hour.unwrap()).collect();
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
        err.to_string().contains("framing stopped before physical EOF"),
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
    assert!(hours.is_empty(), "corruption before the first FRAME_END emits no hours");
    assert!(raw_cursor.stats().stop_reason.is_some());
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
        tail_error: Some((io::ErrorKind::InvalidData, "simulated zstd checksum mismatch")),
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
        err.to_string().contains("framing stopped before physical EOF"),
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
    let index = cursor.take_index();
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
