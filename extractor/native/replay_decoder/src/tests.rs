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
use std::io::Cursor;
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
