use super::*;
use compact_contract::calendar::DATE_EPOCH_CONST;
use compact_contract::campaign_identity::{
    encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
    COMPACT_RECORD_TYPE_STREAM_HEADER,
};
use compact_contract::{
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
};
use recorder_host::world_fold::WorldFold;
use std::io::{Cursor, Read};
use std::path::PathBuf;

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
        p.push(1);
    }
    p
}

fn frame_end_payload() -> Vec<u8> {
    vec![0u8; 32]
}

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

const CAMPAIGN_UUID: &str = "a4b17fed-ed12-4361-9064-fb6576638110";

fn identity_v1_stream() -> Vec<u8> {
    identity_v1_stream_with_seed(1_026_046_527)
}

fn identity_v1_stream_with_seed(seed: i32) -> Vec<u8> {
    let observation = Cid1Observation {
        game_unique_id: CAMPAIGN_UUID.to_string(),
        game_unique_seed: seed,
        game_hour_raw: raw_for(HOUR_A),
    };
    let fields = Cid1Fields::from_stable_observations(
        observation.clone(),
        observation,
        "vanilla_latest_2026_07_20".to_string(),
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
    stream.extend_from_slice(&three_hour_stream());
    stream
}

fn reference_fold(stream: &[u8], game_id: &str) -> Vec<(i64, String, Vec<u8>)> {
    let mut fold = WorldFold::new(game_id);
    fold.ingest_stream(stream);
    fold.finish()
}

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
    std::fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../snapshot/tests/data/legacy/pre222_store_deflate.battleplan"),
    )
    .expect("canonicalize checked-in legacy replay fixture")
}

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
    assert_eq!(stats.per_domain_data_records.get(&18), Some(&3));
    assert_eq!(got[0].domain_data_counts.get(&18), Some(&1));
}

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

#[test]
fn truncated_tail_yields_complete_hours_only() {
    let mut stream = three_hour_stream();
    stream.truncate(stream.len() - 5);
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
fn malformed_domain_payload_is_excluded_and_structural_stream_certifies() {
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
    cursor
        .stats()
        .certification_verdict()
        .expect("a closed, framed stream remains structurally certifiable");
    assert_eq!(cursor.stats().excluded_domain_data_records.len(), 1);
    let excluded = &cursor.stats().excluded_domain_data_records[0];
    assert_eq!(excluded.stream_offset, 0);
    assert_eq!(excluded.game_hour_raw, raw_for(HOUR_A));
    assert_eq!(excluded.domain_id, 18);
    assert!(!excluded.validation_reason.is_empty(), "{excluded:?}");
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
        err.contains("final record must be a canonical FRAME_END or FRAME_REFUSED boundary"),
        "{err}"
    );
}

#[test]
fn a_stream_that_ends_on_a_canonical_frame_refused_certifies() {
    let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: raw_for(HOUR_A + 1),
        offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
            domain_id: 64,
            status_flags: 0x0000_0100,
            non_tolerated_flags: 0x0000_0100,
            predicate: compact_contract::diagnostic_channel::Predicate::new(
                1,
                "synthetic_domain_read_is_bound",
            )
            .unwrap(),
            authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
            off_frame: None,
            reason: "d64_rejected_timed_effect_frames=1".to_string(),
            reason_units_named: 0,
        }],
        frame_findings: Vec::new(),
    }
    .encode()
    .expect("canonical refusal fixture");
    let mut refused = Vec::new();
    push_record(
        &mut refused,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut refused,
        2,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    push_record(
        &mut refused,
        3,
        raw_for(HOUR_A + 1),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut refused,
        4,
        raw_for(HOUR_A + 1),
        DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal,
    );
    let mut cursor = open_reader(
        Cursor::new(refused.clone()),
        refused.len() as u64,
        "fold_check",
    );
    assert_eq!(cursor.by_ref().count(), 1);
    cursor
        .stats()
        .certification_verdict()
        .expect("a stream ending on a canonical FRAME_REFUSED closes cleanly");
}

#[test]
fn a_refused_hour_does_not_carry_its_counts_into_the_next_hour() {
    let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: raw_for(HOUR_A),
        offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
            domain_id: 64,
            status_flags: 0x0000_0100,
            non_tolerated_flags: 0x0000_0100,
            predicate: compact_contract::diagnostic_channel::Predicate::new(
                1,
                "synthetic_domain_read_is_bound",
            )
            .unwrap(),
            authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
            off_frame: None,
            reason: "d64_rejected_timed_effect_frames=1".to_string(),
            reason_units_named: 0,
        }],
        frame_findings: Vec::new(),
    }
    .encode()
    .expect("canonical refusal fixture");
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    let hour_b_first_record_offset =
        stream.len() as u64 + (COMPACT_RECORD_HEADER_LEN + refusal.len()) as u64;
    push_record(
        &mut stream,
        2,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal,
    );
    push_record(
        &mut stream,
        3,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut stream,
        4,
        raw_for(HOUR_B),
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
        .map(|hour| hour.expect("no io error"))
        .collect::<Vec<_>>();
    cursor
        .stats()
        .certification_verdict()
        .expect("a refusal followed by a clean closed hour certifies");
    assert_eq!(
        hours.len(),
        1,
        "only the published hour is emitted; the refused hour published nothing"
    );
    assert_eq!(hours[0].hour_index, HOUR_B);
    assert_eq!(
        hours[0].domain_data_counts.get(&18),
        Some(&1),
        "hour {} carried exactly one d18 record; the refused hour's record must not be added to it: {:?}",
        HOUR_B,
        hours[0].domain_data_counts
    );
    let dir = TmpDir::new("refused_hour_counts");
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, &stream).unwrap();
    let index = build_index(&Source::StreamFile(path), &OpenOptions::default())
        .expect("a refusal followed by a clean closed hour indexes");
    assert_eq!(index.hours.len(), 1);
    assert_eq!(index.hours[0].hour_index, HOUR_B);
    assert_eq!(
        index.hours[0].start_offset, 0,
        "hour {HOUR_B}'s replay range must still cover the refused hour's folded records"
    );
    assert!(index.hours[0].start_offset < hour_b_first_record_offset);
    assert!(index.hours[0].cheap_seek_point);
}

#[test]
fn a_refusal_naming_a_later_hour_cannot_close_an_open_one() {
    let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: raw_for(HOUR_A + 1),
        offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
            domain_id: 64,
            status_flags: 0x0000_0100,
            non_tolerated_flags: 0x0000_0100,
            predicate: compact_contract::diagnostic_channel::Predicate::new(
                1,
                "synthetic_domain_read_is_bound",
            )
            .unwrap(),
            authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
            off_frame: None,
            reason: "d64_rejected_timed_effect_frames=1".to_string(),
            reason_units_named: 0,
        }],
        frame_findings: Vec::new(),
    }
    .encode()
    .expect("canonical refusal fixture");
    let mut abandons = Vec::new();
    push_record(
        &mut abandons,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut abandons,
        2,
        raw_for(HOUR_A + 1),
        DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal,
    );
    let mut cursor = open_reader(
        Cursor::new(abandons.clone()),
        abandons.len() as u64,
        "fold_check",
    );
    cursor.by_ref().for_each(drop);
    let err = cursor
        .stats()
        .certification_verdict()
        .expect_err("an abandoned open hour must fail certification");
    assert!(err.contains("mismatched FRAME_REFUSED"), "{err}");
}

#[test]
fn legacy_container_opens_and_matches_reference() {
    let path = legacy_fixture_path();

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
    let v = hours[0].to_json().expect("snapshot is valid JSON");
    assert_eq!(
        v.get("hour_index").and_then(|x| x.as_i64()),
        Some(16_959_360)
    );
    assert_eq!(v.get("game_id").and_then(|x| x.as_str()), Some("legacy"));

    let container = snapshot::container::Container::open_strict_with_file_receipt_from_file(
        std::fs::File::open(&path).expect("open container directly"),
        &snapshot::container::ExpectedIdentity::default(),
    )
    .map(|(container, _, _)| container)
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

#[test]
fn header_corruption_in_compressed_stream_is_a_hard_decode_error() {
    let dir = TmpDir::new("zstd_header_flip");
    let clean = three_hour_stream();
    let mut corrupted = clean.clone();
    let first_len = compact_contract::parse_compact_record_header(&clean)
        .unwrap()
        .record_len();
    corrupted[first_len] ^= 0xFF;

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

    let mut raw_cursor = Source::detect(&raw_path)
        .unwrap()
        .open_cursor(&opts)
        .unwrap();
    let hours = raw_cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect("raw torn/corrupt tails keep the tolerant stop");
    assert!(
        !hours.is_empty(),
        "the hours after the damage must survive it: {}",
        raw_cursor.stats().hour_coverage.summary()
    );
    assert!(raw_cursor.stats().stop_reason.is_some());
    assert!(
        raw_cursor.stats().certification_verdict().is_err(),
        "diagnostic raw-prefix tolerance must not become certification"
    );
    let excluded = raw_cursor.stats().hour_coverage.excluded_hours();
    assert!(
        excluded
            .iter()
            .any(|(_, reason)| reason.contains("framing damage")),
        "the damaged hour must be excluded and named: {excluded:?}"
    );
    let error = build_index(&Source::detect(&raw_path).unwrap(), &opts)
        .expect_err("hours recovered after framing damage are not vouched for");
    let message = format!("{error}");
    assert!(
        message.contains("nothing to seal") && message.contains("recovered after framing damage"),
        "the refusal must say nothing is verified, and why: {message}"
    );
}

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
    let mut cursor = HourCursor::from_reader_compressed(
        Box::new(reader),
        0,
        "gid".to_string(),
        true,
        StreamIdentityMode::Legacy,
    );
    let err = cursor
        .by_ref()
        .collect::<Result<Vec<WorldHour>, DecodeError>>()
        .expect_err("checksum error during drain must surface");
    assert!(
        err.to_string().contains("simulated zstd checksum mismatch"),
        "root error must propagate, got: {err}"
    );
}

#[test]
fn taking_the_hour_index_charge_transfers_it_instead_of_doubling_it() {
    let mut cursor = HourCursor::from_reader_compressed(
        Box::new(io::Cursor::new(Vec::new())),
        0,
        "gid".to_string(),
        false,
        StreamIdentityMode::Legacy,
    );
    cursor.hour_index_charge.set(4096);

    let handed_on = cursor.take_index_rows_charge();

    assert_eq!(
        cursor.hour_index_charge.bytes(),
        0,
        "the transfer must move the charge out of the cursor, not copy it"
    );
    assert_eq!(handed_on.bytes(), 4096, "the whole charge must arrive");

    // Dropping a cursor that owns nothing releases nothing, and the charge it
    drop(cursor);
    assert_eq!(handed_on.bytes(), 4096);
}

#[test]
fn framing_stop_with_intact_storage_is_itself_the_hard_error() {
    let reader = ScriptedReader {
        prefix: one_record_then_garbage(),
        served: false,
        tail_error: None,
    };
    let mut cursor = HourCursor::from_reader_compressed(
        Box::new(reader),
        0,
        "gid".to_string(),
        true,
        StreamIdentityMode::Legacy,
    );
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

    let mut cursor = source.open_cursor(&opts).unwrap();
    let linear: Vec<WorldHour> = cursor.by_ref().map(|h| h.unwrap()).collect();
    assert_eq!(linear.len(), 3);

    let index = cursor
        .take_index()
        .expect("complete source-backed cursor must mint a certified index");
    assert_eq!(index.hour_range(), Some((HOUR_A, HOUR_C)));
    assert_eq!(index.cheap_seek_points(), vec![HOUR_A]);

    let index2 = build_index(&source, &opts).unwrap();
    assert_eq!(index, index2);

    let json = index.to_json();
    assert_eq!(json["index_schema"], "h4cr-certified-index-v3");
    assert_eq!(json["identity_mode"], "legacy");
    assert_eq!(
        json["excluded_hours"],
        serde_json::json!([]),
        "an uninjured recording states that it refuses nothing, rather than \
         leaving a reader to infer it"
    );
    let round = StreamIndex::from_json(&json).expect("index json round-trip");
    assert_eq!(round, index);

    for retired in ["h4cr-certified-index-v1", "h4cr-certified-index-v2"] {
        let mut older = json.clone();
        older["index_schema"] = serde_json::json!(retired);
        older.as_object_mut().unwrap().remove("excluded_hours");
        assert!(
            StreamIndex::from_json(&older).is_none(),
            "{retired} must be refused, not read as a recording with no holes"
        );
    }
    let mut missing_mode = json.clone();
    missing_mode
        .as_object_mut()
        .unwrap()
        .remove("identity_mode");
    assert!(StreamIndex::from_json(&missing_mode).is_none());

    for want in [HOUR_A, HOUR_B, HOUR_C] {
        let seeked = seek_to_hour(&source, &index, want, &opts).expect("seek");
        let expected = linear.iter().find(|h| h.hour_index == want).unwrap();
        assert_eq!(&seeked, expected, "seek to hour {want} != linear");
    }

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
    let index = build_index(
        &Source::StreamFile(path),
        &OpenOptions {
            game_id: Some("fold_check".to_string()),
        },
    )
    .expect("the hours before a malformed suffix are verified and indexable");
    assert_eq!(index.hours.len(), 3, "three hours were framed end to end");
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
        Err(SeekError::Open(OpenError::Certification(_))) | Err(SeekError::InvalidIndex(_)) => {}
        other => panic!("malformed suffix must prevent seek result: {other:?}"),
    }

    let mut prefix_copy_mutation = stream;
    let fra = prefix_copy_mutation
        .windows(3)
        .rposition(|window| window == b"FRA")
        .expect("third-hour FRA row exists");
    prefix_copy_mutation[fra..fra + 3].copy_from_slice(b"ITA");
    std::fs::write(&original_path, prefix_copy_mutation).unwrap();
    assert_invalid_index(seek_to_hour(&original, &index, HOUR_A, &opts));
}

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
    assert!(meta.domain_name(53).is_some());
    for d in &meta.domains {
        assert_ne!(d.id, 0);
        assert!(!d.wire_name.is_empty());
    }
}

#[test]
fn identity_v1_cursor_retains_cid1_and_excludes_it_from_ordinary_dispatch() {
    let stream = identity_v1_stream();
    let mut cursor = open_reader_with_identity_mode(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
        StreamIdentityMode::IdentityV1,
    );
    assert_eq!(cursor.identity_mode(), StreamIdentityMode::IdentityV1);
    assert!(cursor.campaign_identity().is_none());

    let hours = cursor.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(hours.len(), 3);
    let identity = cursor.campaign_identity().expect("CID1 retained");
    assert_eq!(identity.game_unique_id(), CAMPAIGN_UUID);
    assert_eq!(identity.game_unique_seed(), 1_026_046_527);
    assert_eq!(identity.game_hour_raw(), raw_for(HOUR_A));
    assert_eq!(cursor.stats().records_total, 6);
    assert_eq!(cursor.stats().data_records, 3);
    assert_eq!(cursor.stats().frame_end_records, 3);
    assert_eq!(cursor.stats().other_records, 0);
    assert!(cursor.stats().certification_verdict().is_ok());
    cursor.certify_complete().unwrap();
}

#[test]
fn identity_cursor_names_freeze_census_without_changing_world_state() {
    use compact_contract::freeze_census_record::{
        FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
    };
    let with_tail = identity_v1_stream();
    let cid = compact_contract::parse_compact_record_header(&with_tail).unwrap();
    let cid_bytes = &with_tail[..cid.record_len()];
    let hour = raw_for(HOUR_A);

    let mut without = cid_bytes.to_vec();
    push_record(
        &mut without,
        1,
        hour,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut without,
        2,
        hour,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let census = FreezeCensusRecordV1::encode_bounded(
        FreezeCensusPhase::Install,
        FreezeCensusCounterSnapshot::default(),
        FreezeCensusCounterSnapshot::default(),
        &[],
    )
    .unwrap();
    let mut with = cid_bytes.to_vec();
    push_record(
        &mut with,
        1,
        0,
        DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
        &census,
    );
    push_record(
        &mut with,
        2,
        hour,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut with,
        3,
        hour,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut plain = open_reader_with_identity_mode(
        Cursor::new(without.clone()),
        without.len() as u64,
        "fold_check",
        StreamIdentityMode::IdentityV1,
    );
    let mut censused = open_reader_with_identity_mode(
        Cursor::new(with.clone()),
        with.len() as u64,
        "fold_check",
        StreamIdentityMode::IdentityV1,
    );
    let plain_hours = plain.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    let censused_hours = censused.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(plain_hours[0].snapshot, censused_hours[0].snapshot);
    assert_eq!(censused.stats().freeze_census_records, 1);
    assert_eq!(censused.stats().other_records, 0);
    assert!(censused.stats().certification_verdict().is_ok());
}

#[test]
fn the_replay_decoder_reads_an_epilogue_carrying_recording_identically() {
    use compact_contract::freeze_census_record::{
        FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
    };
    let cid = compact_contract::parse_compact_record_header(&identity_v1_stream()).unwrap();
    let with_tail = identity_v1_stream();
    let cid_bytes = &with_tail[..cid.record_len()];
    let hour = raw_for(HOUR_A);

    let mut plain_bytes = cid_bytes.to_vec();
    push_record(
        &mut plain_bytes,
        1,
        hour,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut plain_bytes,
        2,
        hour,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let release = FreezeCensusRecordV1::encode_bounded(
        FreezeCensusPhase::Release,
        FreezeCensusCounterSnapshot::default(),
        FreezeCensusCounterSnapshot::default(),
        &[],
    )
    .unwrap();
    let mut epilogue_bytes = plain_bytes.clone();
    push_record(
        &mut epilogue_bytes,
        3,
        0,
        DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
        &release,
    );

    let read = |bytes: Vec<u8>| {
        let len = bytes.len() as u64;
        let mut reader = open_reader_with_identity_mode(
            Cursor::new(bytes),
            len,
            "fold_check",
            StreamIdentityMode::IdentityV1,
        );
        let hours = reader.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
        let stats = reader.stats().clone();
        (hours, stats)
    };
    let (plain_hours, plain_stats) = read(plain_bytes);
    let (epilogue_hours, epilogue_stats) = read(epilogue_bytes);

    assert_eq!(plain_hours.len(), epilogue_hours.len());
    assert_eq!(plain_hours[0].snapshot, epilogue_hours[0].snapshot);
    assert_eq!(
        plain_stats.frame_end_records,
        epilogue_stats.frame_end_records
    );
    assert_eq!(plain_stats.data_records, epilogue_stats.data_records);
    assert_eq!(epilogue_stats.freeze_census_records, 1);
    assert_eq!(epilogue_stats.other_records, 0);
    assert!(
        epilogue_stats.certification_verdict().is_ok(),
        "{:?}",
        epilogue_stats.certification_verdict()
    );
}

#[test]
fn the_replay_decoder_refuses_every_other_shape_after_the_final_boundary() {
    use compact_contract::freeze_census_record::{
        FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
    };
    let with_tail = identity_v1_stream();
    let cid = compact_contract::parse_compact_record_header(&with_tail).unwrap();
    let cid_bytes = &with_tail[..cid.record_len()];
    let hour = raw_for(HOUR_A);
    let release = FreezeCensusRecordV1::encode_bounded(
        FreezeCensusPhase::Release,
        FreezeCensusCounterSnapshot::default(),
        FreezeCensusCounterSnapshot::default(),
        &[],
    )
    .unwrap();

    let one_closed_hour = || {
        let mut bytes = cid_bytes.to_vec();
        push_record(
            &mut bytes,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut bytes,
            2,
            hour,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        bytes
    };
    let push_epilogue = |bytes: &mut Vec<u8>, sequence: u64| {
        push_record(
            bytes,
            sequence,
            0,
            DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            &release,
        );
    };
    let refuses = |bytes: Vec<u8>, what: &str| {
        let len = bytes.len() as u64;
        let mut reader = open_reader_with_identity_mode(
            Cursor::new(bytes),
            len,
            "fold_check",
            StreamIdentityMode::IdentityV1,
        );
        let drained = reader.by_ref().collect::<Result<Vec<_>, _>>();
        let refused = drained.is_err() || reader.stats().certification_verdict().is_err();
        assert!(refused, "{what} was accepted");
    };

    let mut two = one_closed_hour();
    push_epilogue(&mut two, 3);
    push_epilogue(&mut two, 4);
    refuses(two, "a second teardown epilogue");

    let mut then_data = one_closed_hour();
    push_epilogue(&mut then_data, 3);
    push_record(
        &mut then_data,
        4,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    refuses(then_data, "a record after the teardown epilogue");

    let mut no_boundary = cid_bytes.to_vec();
    push_record(
        &mut no_boundary,
        1,
        hour,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_epilogue(&mut no_boundary, 2);
    refuses(
        no_boundary,
        "a teardown epilogue with no closed hour before it",
    );
}

#[test]
fn rollover_stream_set_authenticates_every_cid1_before_yielding_world_hours() {
    let dir = TmpDir::new("rollover_set");
    let paths = [dir.0.join("stream-0.h4cr"), dir.0.join("stream-1.h4cr")];
    let streams = [
        identity_v1_stream_with_seed(1_026_046_527),
        identity_v1_stream_with_seed(1_026_046_528),
    ];
    let mut descriptors = Vec::new();
    for (logical_stream_id, (path, bytes)) in paths.iter().zip(streams.iter()).enumerate() {
        std::fs::write(path, bytes).unwrap();
        let identity = read_placement_authenticated_cid1(path).unwrap();
        descriptors.push(RolloverStreamDescriptor {
            logical_stream_id: logical_stream_id as u64,
            path: path.clone(),
            expected_last_complete_sequence: 6,
            expected_campaign_id: identity.campaign_id(),
            expected_cid1_payload_sha256: identity.payload_sha256(),
            expected_file_sha256: Sha256::digest(bytes),
        });
    }
    let transition = RolloverTransitionDescriptor {
        current_logical_stream_id: 0,
        proposed_logical_stream_id: 1,
        last_admitted_h4cr_sequence: descriptors[0].expected_last_complete_sequence,
        boundary_game_hour_raw: raw_for(HOUR_A),
        reason: RolloverReason::WholePairChanged,
        proposal_nonce: 1,
        current_campaign_id: descriptors[0].expected_campaign_id,
        proposed_campaign_id: descriptors[1].expected_campaign_id,
        current_cid1_payload_sha256: descriptors[0].expected_cid1_payload_sha256,
        proposed_cid1_payload_sha256: descriptors[1].expected_cid1_payload_sha256,
    };
    let descriptor = RolloverSetDescriptor {
        ordered_streams: descriptors,
        transitions: vec![transition],
    };
    let mut set = open_rollover_stream_set(
        descriptor.clone(),
        OpenOptions {
            game_id: Some("rollover-set".to_string()),
        },
    )
    .unwrap();
    assert_eq!(set.identities().len(), 2);
    assert_eq!(set.current_logical_stream_id(), Some(0));
    let hours = std::iter::from_fn(|| set.next_hour())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(hours.len(), 6);
    assert_eq!(
        hours
            .iter()
            .map(|hour| hour.logical_stream_id)
            .collect::<Vec<_>>(),
        vec![0, 0, 0, 1, 1, 1]
    );
    assert_eq!(hours[0].hour.hour_index, HOUR_A);
    assert_eq!(hours[3].hour.hour_index, HOUR_A);
    assert_eq!(set.current_logical_stream_id(), None);

    let mut mismatched_receipt = descriptor;
    mismatched_receipt.ordered_streams[1].expected_cid1_payload_sha256[0] ^= 1;
    let error = open_rollover_stream_set(mismatched_receipt, OpenOptions::default())
        .err()
        .expect("receipt mismatch must fail before a cursor is returned");
    assert!(error.to_string().contains("durable receipt"));
}

#[test]
fn rollover_stream_set_rejects_nonadjacent_transition_before_yielding() {
    let dir = TmpDir::new("rollover_transition_chain");
    let paths = [dir.0.join("stream-0.h4cr"), dir.0.join("stream-1.h4cr")];
    let streams = [
        identity_v1_stream_with_seed(1_026_046_527),
        identity_v1_stream_with_seed(1_026_046_528),
    ];
    let mut ordered_streams = Vec::new();
    for (logical_stream_id, (path, bytes)) in paths.iter().zip(streams.iter()).enumerate() {
        std::fs::write(path, bytes).unwrap();
        let identity = read_placement_authenticated_cid1(path).unwrap();
        ordered_streams.push(RolloverStreamDescriptor {
            logical_stream_id: logical_stream_id as u64,
            path: path.clone(),
            expected_last_complete_sequence: 6,
            expected_campaign_id: identity.campaign_id(),
            expected_cid1_payload_sha256: identity.payload_sha256(),
            expected_file_sha256: Sha256::digest(bytes),
        });
    }
    let transition = RolloverTransitionDescriptor {
        current_logical_stream_id: 0,
        proposed_logical_stream_id: 1,
        last_admitted_h4cr_sequence: 6,
        boundary_game_hour_raw: raw_for(HOUR_A),
        reason: RolloverReason::WholePairChanged,
        proposal_nonce: 1,
        current_campaign_id: ordered_streams[0].expected_campaign_id,
        proposed_campaign_id: ordered_streams[1].expected_campaign_id,
        current_cid1_payload_sha256: ordered_streams[0].expected_cid1_payload_sha256,
        proposed_cid1_payload_sha256: ordered_streams[1].expected_cid1_payload_sha256,
    };
    let valid = RolloverSetDescriptor {
        ordered_streams,
        transitions: vec![transition],
    };

    let mut forged_member_frontier = valid.clone();
    forged_member_frontier.ordered_streams[0].expected_last_complete_sequence = 5;
    forged_member_frontier.transitions[0].last_admitted_h4cr_sequence = 5;
    let error = open_rollover_stream_set(forged_member_frontier, OpenOptions::default())
        .err()
        .expect("forged member frontier must fail full-stream pre-admission");
    assert!(error.to_string().contains("durable frontier"), "{error}");

    let mut wrong_frontier = valid.clone();
    wrong_frontier.transitions[0].last_admitted_h4cr_sequence = 5;
    let error = open_rollover_stream_set(wrong_frontier, OpenOptions::default())
        .err()
        .expect("forged predecessor frontier must fail");
    assert!(error.to_string().contains("not adjacent"), "{error}");

    let mut wrong_boundary = valid.clone();
    wrong_boundary.transitions[0].boundary_game_hour_raw = raw_for(HOUR_B);
    let error = open_rollover_stream_set(wrong_boundary, OpenOptions::default())
        .err()
        .expect("boundary/CID1 mismatch must fail");
    assert!(error.to_string().contains("boundary hour"), "{error}");

    let mut wrong_reason = valid;
    wrong_reason.transitions[0].reason = RolloverReason::SamePairTimeRollback;
    let error = open_rollover_stream_set(wrong_reason, OpenOptions::default())
        .err()
        .expect("CRP1 reason/campaign mismatch must fail");
    assert!(error.to_string().contains("CRP1 semantics"), "{error}");
}

#[test]
fn rollover_stream_set_requires_strictly_increasing_transition_nonces() {
    let dir = TmpDir::new("rollover_nonce_chain");
    let mut ordered_streams = Vec::new();
    for (logical_stream_id, seed) in [1_026_046_527, 1_026_046_528, 1_026_046_529]
        .into_iter()
        .enumerate()
    {
        let path = dir.0.join(format!("stream-{logical_stream_id}.h4cr"));
        let bytes = identity_v1_stream_with_seed(seed);
        std::fs::write(&path, &bytes).unwrap();
        let identity = read_placement_authenticated_cid1(&path).unwrap();
        ordered_streams.push(RolloverStreamDescriptor {
            logical_stream_id: logical_stream_id as u64,
            path,
            expected_last_complete_sequence: 6,
            expected_campaign_id: identity.campaign_id(),
            expected_cid1_payload_sha256: identity.payload_sha256(),
            expected_file_sha256: Sha256::digest(&bytes),
        });
    }
    let transitions = (0..2)
        .map(|index| RolloverTransitionDescriptor {
            current_logical_stream_id: index as u64,
            proposed_logical_stream_id: index as u64 + 1,
            last_admitted_h4cr_sequence: 6,
            boundary_game_hour_raw: raw_for(HOUR_A),
            reason: RolloverReason::WholePairChanged,
            proposal_nonce: 1,
            current_campaign_id: ordered_streams[index].expected_campaign_id,
            proposed_campaign_id: ordered_streams[index + 1].expected_campaign_id,
            current_cid1_payload_sha256: ordered_streams[index].expected_cid1_payload_sha256,
            proposed_cid1_payload_sha256: ordered_streams[index + 1].expected_cid1_payload_sha256,
        })
        .collect();
    let error = open_rollover_stream_set(
        RolloverSetDescriptor {
            ordered_streams,
            transitions,
        },
        OpenOptions::default(),
    )
    .err()
    .expect("repeated accepted proposal nonce must fail");
    assert!(error.to_string().contains("strictly increasing"), "{error}");
}

#[test]
fn identity_v1_index_binds_mode_and_seek_reopens_under_it() {
    let dir = TmpDir::new("identity_index");
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, identity_v1_stream()).unwrap();
    let source = Source::StreamFile(path);
    let opts = OpenOptions {
        game_id: Some("fold_check".to_string()),
    };
    let index =
        build_index_with_identity_mode(&source, &opts, StreamIdentityMode::IdentityV1).unwrap();
    assert_eq!(index.identity_mode, StreamIdentityMode::IdentityV1);
    assert_eq!(index.to_json()["identity_mode"], "identity_v1");
    assert_eq!(
        seek_to_hour(&source, &index, HOUR_B, &opts)
            .unwrap()
            .hour_index,
        HOUR_B
    );
}

#[test]
fn identity_protocol_selection_never_sniffs_or_falls_back() {
    let identity = identity_v1_stream();
    let mut legacy = open_reader(
        Cursor::new(identity.clone()),
        identity.len() as u64,
        "fold_check",
    );
    assert_eq!(legacy.by_ref().count(), 0);
    assert!(legacy.campaign_identity().is_none());
    assert!(legacy.stats().certification_verdict().is_err());

    let plain = three_hour_stream();
    let mut identity_v1 = open_reader_with_identity_mode(
        Cursor::new(plain.clone()),
        plain.len() as u64,
        "fold_check",
        StreamIdentityMode::IdentityV1,
    );
    assert_eq!(identity_v1.by_ref().count(), 0);
    assert!(identity_v1.campaign_identity().is_none());
    let error = identity_v1
        .stats()
        .boundary_error
        .as_deref()
        .expect("mode mismatch is terminal");
    assert!(error.contains("invalid CID1 stream header"), "{error}");
}

#[test]
fn invalid_cid1_never_reaches_boundary_counts_or_world_fold() {
    let mut stream = identity_v1_stream();
    stream[COMPACT_RECORD_HEADER_LEN] = b'X';
    let mut cursor = open_reader_with_identity_mode(
        Cursor::new(stream.clone()),
        stream.len() as u64,
        "fold_check",
        StreamIdentityMode::IdentityV1,
    );
    assert_eq!(cursor.by_ref().count(), 0);
    assert!(cursor.campaign_identity().is_none());
    assert_eq!(cursor.stats().records_total, 0);
    assert_eq!(cursor.stats().hours_emitted, 0);
    assert!(cursor.stats().certification_verdict().is_err());
}

#[test]
fn certified_raw_h4cr_produces_the_neutral_prefolded_contract() {
    let dir = TmpDir::new("neutral_prefolded");
    let path = dir.0.join("collector_stream.bin");
    std::fs::write(&path, three_hour_stream()).unwrap();
    let replay = fold_raw_h4cr_prefolded(
        &path,
        &OpenOptions {
            game_id: Some("fold_check".to_string()),
        },
    )
    .unwrap();

    assert_eq!(replay.index().game_id(), "fold_check");
    assert_eq!(replay.index().schema_version(), &serde_json::json!("1"));
    assert_eq!(replay.index().hours(), &[HOUR_A, HOUR_B, HOUR_C]);
    assert_eq!(replay.entries().len(), 3);
    assert_eq!(
        replay.entry(HOUR_B).unwrap().snapshot().hour_index,
        Some(HOUR_B)
    );
}

#[test]
fn certified_binary_artifacts_reconstruct_every_hour_and_bound_all_seeks() {
    let dir = TmpDir::new("certified_binary_artifacts");
    let source = dir.0.join("collector_stream.bin");
    let artifact_root = dir.0.join("active_artifact_spill");
    std::fs::create_dir(&artifact_root).unwrap();
    let stream = three_hour_stream();
    std::fs::write(&source, &stream).unwrap();
    let source_sha256 = to_hex(&Sha256::digest(&stream));
    let reference = reference_fold(&stream, "fold_check");

    let artifacts = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "fold_check",
        StreamIdentityMode::Legacy,
        stream.len() as u64,
        &source_sha256,
        2,
    )
    .unwrap();

    assert_eq!(artifacts.hour_count, reference.len() as u64);
    assert_eq!(artifacts.checkpoint_count, 2);
    assert_eq!(artifacts.max_seek_deltas, 1);
    assert!(
        artifact_root.is_dir(),
        "finalization must retain the active binary artifact spill"
    );
    let index_bytes = std::fs::read(&artifacts.hour_index_path).unwrap();
    let cache_bytes = std::fs::read(&artifacts.checkpoint_cache_path).unwrap();
    let cache = snapshot::CertifiedReplayCache::open(
        &index_bytes,
        cache_bytes,
        stream.len() as u64,
        &source_sha256,
    )
    .unwrap();
    assert_eq!(
        cache
            .index()
            .hours()
            .iter()
            .map(snapshot::ReplayHourEntry::hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_A, HOUR_B, HOUR_C]
    );
    assert_eq!(
        cache.index().hours().first().unwrap().h4cr_start_offset(),
        0
    );
    assert_eq!(
        cache.index().hours().last().unwrap().h4cr_end_offset(),
        stream.len() as u64
    );
    for (hour, (_, _, expected_json)) in cache.index().hours().iter().zip(reference.iter()) {
        let (actual_json, receipt) = cache.reconstruct_json(hour.hour_index()).unwrap();
        assert_eq!(&actual_json, expected_json);
        assert!(receipt.applied_deltas <= 1);
    }
    let mut pending_dirs = vec![artifact_root.clone()];
    let mut legacy_snapshots = 0;
    while let Some(current) = pending_dirs.pop() {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending_dirs.push(entry.path());
            } else if entry
                .file_name()
                .to_string_lossy()
                .ends_with(".snapshot.json")
            {
                legacy_snapshots += 1;
            }
        }
    }
    assert_eq!(
        legacy_snapshots, 0,
        "production artifact generation must not write per-hour JSON snapshots"
    );
}

/// must not release them.
/// terminal refusal whose bytes CROSS a storage-frame boundary, so a release
#[test]
fn a_terminal_frame_refusal_keeps_its_bytes_in_the_spool() {
    let dir = TmpDir::new("terminal_refusal_trim");

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
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let accepted_hour_end = stream.len() as u64;

    let mut sequence = 3u64;
    for _ in 0..64 {
        push_record(
            &mut stream,
            sequence,
            raw_for(HOUR_A + 1),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER"), (48, "FRA")]),
        );
        sequence += 1;
    }
    let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: raw_for(HOUR_A + 1),
        offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
            domain_id: 64,
            status_flags: 0x0000_0100,
            non_tolerated_flags: 0x0000_0100,
            predicate: compact_contract::diagnostic_channel::Predicate::new(
                1,
                "synthetic_domain_read_is_bound",
            )
            .unwrap(),
            authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
            off_frame: None,
            reason: "d64_rejected_timed_effect_frames=1".to_string(),
            reason_units_named: 0,
        }],
        frame_findings: Vec::new(),
    }
    .encode()
    .expect("canonical refusal fixture");
    push_record(
        &mut stream,
        sequence,
        raw_for(HOUR_A + 1),
        DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal,
    );
    let decoded_len = stream.len() as u64;
    assert!(
        decoded_len > accepted_hour_end,
        "the refused hour must contribute bytes"
    );

    // span, so the false watermark would have released refused bytes.
    let frame_bytes = accepted_hour_end + (decoded_len - accepted_hour_end) / 2;
    let spool = dir.0.join("collector_stream.bin");
    let mut writer = compact_stream::StreamWriter::create_with_frames(
        &spool,
        compact_stream::StreamCompression::Zstd,
        Some(frame_bytes),
    )
    .unwrap();
    writer.write_all(&stream).unwrap();
    let write_receipt = writer.finish().unwrap();
    let crossing = write_receipt
        .frame_boundaries
        .iter()
        .find(|boundary| {
            boundary.decoded_offset > accepted_hour_end && boundary.decoded_offset < decoded_len
        })
        .copied()
        .expect("a storage frame must close inside the refused hour");

    let artifact_root = dir.0.join("artifacts");
    std::fs::create_dir(&artifact_root).unwrap();
    let source_sha256 = to_hex(&Sha256::digest(&stream));
    let artifacts = build_certified_replay_artifacts(
        &spool,
        &artifact_root,
        "fold_check",
        StreamIdentityMode::Legacy,
        decoded_len,
        &source_sha256,
        168,
    )
    .expect("a stream ending on a canonical refusal still certifies");

    assert_eq!(artifacts.hour_count, 1);
    assert_eq!(
        artifacts.durable_decoded_end_offset, accepted_hour_end,
        "published coverage must stop at the accepted hour, not at EOF"
    );
    assert!(
        artifacts.durable_decoded_end_offset < decoded_len,
        "the fixture must actually exercise the gap"
    );

    let receipt = recorder_host::spool_trim::DurableHourReceipt {
        checkpoint_cache_path: artifacts.checkpoint_cache_path.clone(),
        hour_index_path: artifacts.hour_index_path.clone(),
        decoded_len,
        decoded_sha256: source_sha256.clone(),
        published_coverage_path: artifacts.published_coverage_path.clone(),
        fold_covered_decoded_end: artifacts.durable_decoded_end_offset,
    };

    // inside the refused hour would have been selected and released.
    let verified = recorder_host::spool_trim::verify_durable_receipt(&receipt).unwrap();
    assert_eq!(verified.indexed_decoded_end_offset, decoded_len);
    assert_eq!(verified.durable_decoded_end_offset, accepted_hour_end);
    let frames = recorder_host::spool_trim::SpoolFrames::from_receipt(&write_receipt);
    assert_eq!(
        frames
            .releasable_boundary(verified.indexed_decoded_end_offset)
            .map(|boundary| boundary.decoded_offset),
        Some(crossing.decoded_offset),
        "the pre-fix watermark would have chosen the boundary inside the refused hour"
    );

    let outcome = recorder_host::spool_trim::trim_spool_prefix(&spool, &frames, &receipt).unwrap();
    match &outcome {
        recorder_host::spool_trim::TrimOutcome::NothingReleasable { reason } => {
            assert!(reason.contains("no storage frame boundary"), "{reason}");
        }
        recorder_host::spool_trim::TrimOutcome::Released { journal, .. } => {
            assert!(
                journal.live_decoded_offset <= accepted_hour_end,
                "released decoded bytes {} reach into the refused hour that starts at {}",
                journal.live_decoded_offset,
                accepted_hour_end
            );
        }
    }

    // Whatever was released, every refused byte is still readable from the
    let (offset, compression) = match &outcome {
        recorder_host::spool_trim::TrimOutcome::Released { journal, .. } => {
            journal.live_region().unwrap()
        }
        _ => (0, compact_stream::StreamCompression::Zstd),
    };
    let mut surviving = Vec::new();
    compact_stream::open_with_compression_at(&spool, compression, offset)
        .unwrap()
        .read_to_end(&mut surviving)
        .unwrap();
    let surviving_start = match &outcome {
        recorder_host::spool_trim::TrimOutcome::Released { journal, .. } => {
            journal.live_decoded_offset as usize
        }
        _ => 0,
    };
    assert_eq!(surviving, stream[surviving_start..]);
    assert!(
        surviving_start <= accepted_hour_end as usize,
        "the refused hour's bytes must all survive"
    );
    assert!(
        surviving.ends_with(&stream[accepted_hour_end as usize..]),
        "the whole refused span must still be in the spool"
    );
}

#[test]
fn published_coverage_is_conservative_for_every_unpublished_tail() {
    let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: raw_for(HOUR_B),
        offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
            domain_id: 64,
            status_flags: 0x0000_0100,
            non_tolerated_flags: 0x0000_0100,
            predicate: compact_contract::diagnostic_channel::Predicate::new(
                1,
                "synthetic_domain_read_is_bound",
            )
            .unwrap(),
            authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
            off_frame: None,
            reason: "d64_rejected_timed_effect_frames=1".to_string(),
            reason_units_named: 0,
        }],
        frame_findings: Vec::new(),
    }
    .encode()
    .expect("canonical refusal fixture");

    let mut mid = Vec::new();
    let mut sequence = 1u64;
    let mut push_hour = |bytes: &mut Vec<u8>, hour: i64, sequence: &mut u64| {
        push_record(
            bytes,
            *sequence,
            raw_for(hour),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        *sequence += 1;
        push_record(
            bytes,
            *sequence,
            raw_for(hour),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        *sequence += 1;
    };
    push_hour(&mut mid, HOUR_A, &mut sequence);
    push_record(
        &mut mid,
        sequence,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    sequence += 1;
    push_record(
        &mut mid,
        sequence,
        raw_for(HOUR_B),
        DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED,
        &refusal,
    );
    sequence += 1;
    push_hour(&mut mid, HOUR_C, &mut sequence);

    let mut cursor = open_reader(Cursor::new(mid.clone()), mid.len() as u64, "fold_check");
    assert_eq!(cursor.by_ref().count(), 2);
    assert_eq!(
        cursor.published_hour_coverage_end(),
        mid.len() as u64,
        "a mid-stream refusal's bytes are inside the next published hour, so \
         coverage must still reach the decoded end"
    );

    let mut open_tail = Vec::new();
    let mut sequence = 1u64;
    push_hour(&mut open_tail, HOUR_A, &mut sequence);
    let closed_end = open_tail.len() as u64;
    push_record(
        &mut open_tail,
        sequence,
        raw_for(HOUR_B),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );

    let mut cursor = open_reader(
        Cursor::new(open_tail.clone()),
        open_tail.len() as u64,
        "fold_check",
    );
    let _ = cursor.by_ref().count();
    assert_eq!(
        cursor.published_hour_coverage_end(),
        closed_end,
        "an hour left open at EOF published nothing, so its bytes are not covered"
    );
    assert!(cursor.published_hour_coverage_end() < open_tail.len() as u64);
}

#[test]
fn certified_artifacts_are_byte_identical_across_raw_and_compressed_storage() {
    let dir = TmpDir::new("storage_byte_identity");
    let stream = three_hour_stream();
    let source_sha256 = to_hex(&Sha256::digest(&stream));

    let raw_path = dir.0.join("raw_stream.bin");
    std::fs::write(&raw_path, &stream).unwrap();

    let zstd_path = dir.0.join("zstd_stream.bin");
    compact_stream::encode_file(
        &raw_path,
        &zstd_path,
        compact_stream::StreamCompression::Zstd,
    )
    .unwrap();

    // decodable frames so its prefix can later be released.
    let framed_path = dir.0.join("framed_stream.bin");
    let mut framed_writer = compact_stream::StreamWriter::create_with_frames(
        &framed_path,
        compact_stream::StreamCompression::Zstd,
        Some(64),
    )
    .unwrap();
    framed_writer.write_all(&stream).unwrap();
    let framed_receipt = framed_writer.finish().unwrap();
    assert!(
        framed_receipt.frame_boundaries.len() > 2,
        "the framed arm must actually rotate: {:?}",
        framed_receipt.frame_boundaries
    );

    let mut digests = Vec::new();
    for (label, source) in [
        ("raw", &raw_path),
        ("zstd", &zstd_path),
        ("zstd_framed", &framed_path),
    ] {
        let artifact_root = dir.0.join(format!("spill_{label}"));
        std::fs::create_dir(&artifact_root).unwrap();
        let artifacts = build_certified_replay_artifacts(
            source,
            &artifact_root,
            "fold_check",
            StreamIdentityMode::Legacy,
            stream.len() as u64,
            &source_sha256,
            2,
        )
        .unwrap_or_else(|error| panic!("{label} storage failed certification: {error}"));
        assert_eq!(artifacts.source_len, stream.len() as u64);
        assert_eq!(artifacts.source_sha256, source_sha256);
        digests.push((
            label,
            std::fs::read(&artifacts.checkpoint_cache_path).unwrap(),
            std::fs::read(&artifacts.hour_index_path).unwrap(),
            artifacts.checkpoint_cache_sha256.clone(),
            artifacts.hour_index_sha256.clone(),
        ));
    }

    let (_, base_cache, base_index, base_cache_sha, base_index_sha) = digests[0].clone();
    for (label, cache, index, cache_sha, index_sha) in digests.iter().skip(1) {
        assert_eq!(
            cache, &base_cache,
            "{label} spool produced different checkpoints.h4cp bytes"
        );
        assert_eq!(
            index, &base_index,
            "{label} spool produced different hour_index.h4ix bytes"
        );
        assert_eq!(cache_sha, &base_cache_sha);
        assert_eq!(index_sha, &base_index_sha);
    }
    eprintln!(
        "storage_byte_identity checkpoints_sha256={base_cache_sha} hour_index_sha256={base_index_sha}"
    );
}

#[test]
fn compressed_storage_still_enforces_the_decoded_receipt() {
    let dir = TmpDir::new("storage_receipt");
    let stream = three_hour_stream();
    let raw_path = dir.0.join("raw_stream.bin");
    std::fs::write(&raw_path, &stream).unwrap();
    let zstd_path = dir.0.join("zstd_stream.bin");
    compact_stream::encode_file(
        &raw_path,
        &zstd_path,
        compact_stream::StreamCompression::Zstd,
    )
    .unwrap();

    let artifact_root = dir.0.join("spill_wrong_sha");
    std::fs::create_dir(&artifact_root).unwrap();
    let wrong_sha = "0".repeat(64);
    let error = build_certified_replay_artifacts(
        &zstd_path,
        &artifact_root,
        "fold_check",
        StreamIdentityMode::Legacy,
        stream.len() as u64,
        &wrong_sha,
        2,
    )
    .expect_err("a compressed source must still be bound to its decoded sha256");
    assert!(
        error.to_string().contains("certified H4CR identity"),
        "unexpected error: {error}"
    );

    let artifact_root = dir.0.join("spill_wrong_len");
    std::fs::create_dir(&artifact_root).unwrap();
    let error = build_certified_replay_artifacts(
        &zstd_path,
        &artifact_root,
        "fold_check",
        StreamIdentityMode::Legacy,
        stream.len() as u64 + 1,
        &to_hex(&Sha256::digest(&stream)),
        2,
    )
    .expect_err("a compressed source must still be bound to its decoded length");
    assert!(
        error.to_string().contains("certified H4CR identity")
            || error.to_string().contains("not certifiable")
            || error.to_string().contains("does not match"),
        "unexpected error: {error}"
    );

    let artifact_root = dir.0.join("spill_empty");
    std::fs::create_dir(&artifact_root).unwrap();
    let empty_path = dir.0.join("empty.zst");
    std::fs::write(&empty_path, compact_stream::ZSTD_FRAME_MAGIC).unwrap();
    let error = build_certified_replay_artifacts(
        &empty_path,
        &artifact_root,
        "fold_check",
        StreamIdentityMode::Legacy,
        stream.len() as u64,
        &to_hex(&Sha256::digest(&stream)),
        2,
    )
    .expect_err("a truncated compressed source must not certify");
    eprintln!("truncated compressed source rejected: {error}");
}

#[test]
fn portable_decoder_contract_drives_the_same_certified_world_fold() {
    let request =
        snapshot::CertifiedPrefoldRequest::try_new("fold_check", three_hour_stream()).unwrap();
    let replay =
        snapshot::decode_certified_prefolded(&ProductionCertifiedPrefoldDecoder, request).unwrap();

    assert_eq!(replay.index().game_id(), "fold_check");
    assert_eq!(replay.index().hours(), &[HOUR_A, HOUR_B, HOUR_C]);
    assert_eq!(replay.entries().len(), 3);
}

#[test]
fn portable_identity_request_drives_identity_aware_certified_fold() {
    let stream = identity_v1_stream();
    let request = snapshot::CertifiedPrefoldRequest::try_new_with_identity_mode(
        "fold_check",
        stream,
        snapshot::PrefoldStreamIdentityMode::IdentityV1,
    )
    .unwrap();
    let replay =
        snapshot::decode_certified_prefolded(&ProductionCertifiedPrefoldDecoder, request).unwrap();
    assert_eq!(replay.index().hours(), &[HOUR_A, HOUR_B, HOUR_C]);
    assert_eq!(
        replay.receipt().identity_mode(),
        snapshot::PrefoldStreamIdentityMode::IdentityV1
    );
}

#[test]
fn identity_aware_spooled_fold_consumes_cid1_before_file_backed_world_fold() {
    let dir = TmpDir::new("identity_spooled");
    let source = dir.0.join("collector_stream.bin");
    let snapshot_dir = dir.0.join("snapshots");
    std::fs::create_dir(&snapshot_dir).unwrap();
    let stream = identity_v1_stream();
    std::fs::write(&source, &stream).unwrap();
    let sha256 = to_hex(&Sha256::digest(&stream));
    let request = snapshot::CertifiedPrefoldSpoolRequest::try_new_with_identity_mode(
        "fold_check",
        &source,
        &snapshot_dir,
        stream.len() as u64,
        sha256,
        snapshot::PrefoldStreamIdentityMode::IdentityV1,
    )
    .unwrap();
    let spool = fold_raw_h4cr_file_spooled(request).unwrap();
    assert_eq!(spool.entries().len(), 3);
    assert_eq!(
        spool
            .entries()
            .iter()
            .map(|entry| entry.hour_index())
            .collect::<Vec<_>>(),
        vec![HOUR_A, HOUR_B, HOUR_C]
    );
}

#[test]
fn portable_prefold_rejects_closed_hour_the_world_fold_cannot_key() {
    let mut stream = three_hour_stream();
    push_record(
        &mut stream,
        7,
        i32::MAX,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut stream,
        8,
        i32::MAX,
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let error = fold_raw_h4cr_bytes_prefolded(stream, "fold_check").unwrap_err();
    match error {
        OpenError::Certification(message) => {
            assert!(
                message.contains("closed-hour cardinality mismatch"),
                "{message}"
            );
            assert!(message.contains("4 FRAME_END"), "{message}");
            assert!(message.contains("3 WorldFold"), "{message}");
        }
        other => panic!("expected certification refusal, got {other}"),
    }
}

#[test]
fn neutral_prefolded_producer_accepts_only_physically_raw_h4cr() {
    let dir = TmpDir::new("neutral_raw_only");
    for (name, bytes) in [
        ("container.battleplan", b"PK\x03\x04not-a-stream".as_slice()),
        (
            "collector_stream.zst",
            b"\x28\xb5\x2f\xfdnot-raw".as_slice(),
        ),
    ] {
        let path = dir.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(
            fold_raw_h4cr_prefolded(&path, &OpenOptions::default()),
            Err(OpenError::UnrecognizedInput(_))
        ));
    }
}

#[test]
fn a_malformed_raw_suffix_costs_its_own_hours_and_is_named() {
    let dir = TmpDir::new("neutral_no_prefix");
    let path = dir.0.join("collector_stream.bin");
    let mut stream = three_hour_stream();
    stream.extend_from_slice(b"H4C");
    std::fs::write(&path, stream).unwrap();

    let prefolded = fold_raw_h4cr_prefolded(&path, &OpenOptions::default())
        .expect("the intact hours before a malformed suffix are verified and admissible");
    assert_eq!(
        prefolded.entries().len(),
        3,
        "every hour framed end to end is delivered"
    );

    let mut cursor = open(&path).expect("open the same stream");
    cursor.by_ref().for_each(drop);
    let coverage = &cursor.stats().hour_coverage;
    assert_eq!(coverage.verified_count(), 3);
    assert!(
        !coverage.unattributed_losses().is_empty() || !coverage.excluded_hours().is_empty(),
        "the malformed suffix must be NAMED somewhere a consumer reads: {}",
        coverage.summary()
    );
}

#[test]
fn extractor_decoder_manifest_has_no_replay_interface_dependency() {
    let manifest = include_str!("../Cargo.toml").replace('\\', "/");
    assert!(!manifest.contains("replay_interface/"), "{manifest}");
}

fn churning_stream(hours: usize) -> Vec<u8> {
    assert!(hours > 0);
    let tags = [
        "GER", "FRA", "ENG", "SOV", "USA", "ITA", "JAP", "POL", "HOL", "BEL",
    ];
    let mut stream = Vec::new();
    let mut seq = 1u64;
    for hour in 0..hours {
        let width = 3 + (hour % 5);
        let mut rows: Vec<(u32, String)> = Vec::new();
        for offset in 0..width {
            let slot = (hour + offset) % tags.len();
            let mut tag = tags[slot].to_string();
            if hour % 3 == 0 && offset == 1 {
                tag = format!("{}{}", &tags[slot][..2], (b'A' + (hour % 26) as u8) as char);
            }
            rows.push((40 + slot as u32, tag));
        }
        let borrowed = rows
            .iter()
            .map(|(id, tag)| (*id, tag.as_str()))
            .collect::<Vec<_>>();
        push_record(
            &mut stream,
            seq,
            raw_for(HOUR_A + hour as i64),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&borrowed),
        );
        seq += 1;
        push_record(
            &mut stream,
            seq,
            raw_for(HOUR_A + hour as i64),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        seq += 1;
    }
    stream
}

fn legacy_packed_snapshot_series(
    stream_path: &std::path::Path,
    spill: &std::path::Path,
    game_id: &str,
) -> Vec<(i64, Vec<u8>)> {
    let (files, _stats) =
        recorder_host::stream_fold::fold_compact_stream_file_parallel_with_identity_mode(
            stream_path,
            game_id,
            spill,
            recorder_host::stream_fold::default_fold_workers(),
            StreamIdentityMode::Legacy,
        )
        .expect("production file-backed parallel fold");
    files
        .into_iter()
        .map(|file| {
            let bytes = std::fs::read(&file.path).expect("read spilled legacy snapshot");
            (file.hour_index, bytes)
        })
        .collect()
}

fn open_certified_cache(artifacts: &CertifiedReplayArtifacts) -> snapshot::CertifiedReplayCache {
    let index_bytes = std::fs::read(&artifacts.hour_index_path).unwrap();
    let cache_bytes = std::fs::read(&artifacts.checkpoint_cache_path).unwrap();
    snapshot::CertifiedReplayCache::open(
        &index_bytes,
        cache_bytes,
        artifacts.source_len,
        &artifacts.source_sha256,
    )
    .expect("open certified replay cache")
}

#[test]
fn binary_persistence_reconstructs_the_legacy_json_snapshot_series_byte_for_byte() {
    const HOURS: usize = 400;
    const INTERVAL: u32 = 168;

    let dir = TmpDir::new("golden_binary_vs_legacy");
    let source = dir.0.join("collector_stream.bin");
    let stream = churning_stream(HOURS);
    std::fs::write(&source, &stream).unwrap();
    let source_sha256 = to_hex(&Sha256::digest(&stream));

    let legacy_spill = dir.0.join("legacy_spill");
    std::fs::create_dir(&legacy_spill).unwrap();
    let legacy = legacy_packed_snapshot_series(&source, &legacy_spill, "golden_game");
    assert_eq!(
        legacy.len(),
        HOURS,
        "the legacy fold must close one snapshot per synthesized game hour"
    );

    let artifact_root = dir.0.join("artifacts");
    std::fs::create_dir(&artifact_root).unwrap();
    let artifacts = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "golden_game",
        StreamIdentityMode::Legacy,
        stream.len() as u64,
        &source_sha256,
        INTERVAL,
    )
    .expect("build certified replay artifacts");

    assert_eq!(artifacts.hour_count, HOURS as u64);
    assert_eq!(
        artifacts.checkpoint_count,
        HOURS.div_ceil(INTERVAL as usize) as u64
    );
    assert_eq!(artifacts.max_seek_deltas, INTERVAL - 1);

    let cache = open_certified_cache(&artifacts);
    assert_eq!(cache.index().hours().len(), legacy.len());
    for (entry, (legacy_hour, legacy_bytes)) in cache.index().hours().iter().zip(&legacy) {
        assert_eq!(entry.hour_index(), *legacy_hour);
        let (reconstructed, receipt) = cache
            .reconstruct_json(*legacy_hour)
            .unwrap_or_else(|error| panic!("reconstruct hour {legacy_hour}: {error}"));
        assert_eq!(
            &reconstructed, legacy_bytes,
            "hour {legacy_hour} does not reconstruct to the bytes the legacy seal path packs"
        );
        assert!(receipt.applied_deltas < INTERVAL);
    }

    let legacy_bytes: u64 = legacy.iter().map(|(_, bytes)| bytes.len() as u64).sum();
    assert_eq!(artifacts.legacy_json_bytes_avoided, legacy_bytes);
    assert!(
        artifacts.checkpoint_cache_len + artifacts.hour_index_len < legacy_bytes,
        "binary persistence ({} cache + {} index) must be smaller than the {legacy_bytes}-byte JSON series it replaces",
        artifacts.checkpoint_cache_len,
        artifacts.hour_index_len
    );
}

#[test]
fn binary_persistence_reconstruction_is_exact_at_every_checkpoint_distance() {
    const HOURS: usize = 24;
    const INTERVAL: u32 = 24;

    let dir = TmpDir::new("golden_checkpoint_distances");
    let source = dir.0.join("collector_stream.bin");
    let stream = churning_stream(HOURS);
    std::fs::write(&source, &stream).unwrap();
    let source_sha256 = to_hex(&Sha256::digest(&stream));

    let legacy_spill = dir.0.join("legacy_spill");
    std::fs::create_dir(&legacy_spill).unwrap();
    let legacy = legacy_packed_snapshot_series(&source, &legacy_spill, "golden_game");

    let artifact_root = dir.0.join("artifacts");
    std::fs::create_dir(&artifact_root).unwrap();
    let artifacts = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "golden_game",
        StreamIdentityMode::Legacy,
        stream.len() as u64,
        &source_sha256,
        INTERVAL,
    )
    .expect("build certified replay artifacts");
    assert_eq!(artifacts.checkpoint_count, 1);

    let cache = open_certified_cache(&artifacts);
    for (distance, (hour_index, legacy_bytes)) in legacy.iter().enumerate().rev() {
        let (reconstructed, receipt) = cache.reconstruct_json(*hour_index).unwrap();
        assert_eq!(
            &reconstructed, legacy_bytes,
            "reverse-order seek to hour {hour_index} diverged from the legacy fold"
        );
        assert_eq!(receipt.applied_deltas as usize, distance);
        assert_eq!(receipt.checkpoint_hour, legacy[0].0);
    }
}

const T26_HOUR_17032314_FUEL_OBJECT: &str =
    r#"{"cost":0.0,"current":189352.52932739258,"gain":6.103515625,"max":207000.0}"#;

#[test]
fn a_real_captured_hours_float_spelling_survives_a_certified_seek() {
    let reparsed: serde_json::Value = serde_json::from_str(T26_HOUR_17032314_FUEL_OBJECT).unwrap();
    let reserialized = serde_json::to_string(&reparsed).unwrap();
    assert_ne!(
        reserialized, T26_HOUR_17032314_FUEL_OBJECT,
        "the t26 fixture no longer witnesses the round-trip defect"
    );
    assert!(
        reserialized.contains("189352.52932739255"),
        "expected the one-ULP-low spelling, got {reserialized}"
    );

    let bodies = [
        format!(
            r#"{{"fuel":{}}}"#,
            r#"{"cost":0.0,"current":0.0,"gain":0.0,"max":207000.0}"#
        ),
        format!(r#"{{"fuel":{T26_HOUR_17032314_FUEL_OBJECT}}}"#),
        format!(r#"{{"fuel":{T26_HOUR_17032314_FUEL_OBJECT}}}"#),
    ];
    let world_json = |hour: i64, body: &str| {
        format!(
            r#"{{"date":"1936.1.1.{}","game_id":"t26","hour_index":{hour},"schema_version":"1","world":{body}}}"#,
            hour + 1
        )
        .into_bytes()
    };
    let source_sha = to_hex(&Sha256::digest(b"t26 fixture"));
    let mut encoder = snapshot::ReplayCacheEncoder::new(Vec::new(), &source_sha, 2).unwrap();
    for (position, body) in bodies.iter().enumerate() {
        encoder
            .push(snapshot::CacheBuildHour {
                hour_index: 17_032_314 + position as i64,
                date: format!("1936.1.1.{}", 17_032_314 + position as i64 + 1),
                h4cr_start_offset: position as u64 * 100,
                h4cr_end_offset: (position as u64 + 1) * 100,
                world_json: world_json(17_032_314 + position as i64, body),
            })
            .unwrap_or_else(|error| panic!("push hour {position}: {error}"));
    }
    let (cache, index) = encoder
        .finish("t26", "1", snapshot::CacheIdentityMode::Legacy, 300)
        .unwrap();
    let index_bytes = index.to_bytes().unwrap();
    let cache =
        snapshot::CertifiedReplayCache::open(&index_bytes, cache, 300, &source_sha).unwrap();
    for (position, body) in bodies.iter().enumerate() {
        let hour = 17_032_314 + position as i64;
        let (reconstructed, _) = cache.reconstruct_json(hour).unwrap();
        assert_eq!(
            reconstructed,
            world_json(hour, body),
            "hour {hour} did not come back byte-exact"
        );
    }
    assert_eq!(index.hours()[1].delta_steps(), 1);
}

#[test]
fn certified_artifacts_publish_the_retained_campaign_identity() {
    let dir = TmpDir::new("certified_identity_legacy");
    let source = dir.0.join("collector_stream.bin");
    let stream = three_hour_stream();
    std::fs::write(&source, &stream).unwrap();
    let source_sha256 = to_hex(&Sha256::digest(&stream));
    let artifact_root = dir.0.join("artifacts");
    std::fs::create_dir(&artifact_root).unwrap();

    let artifacts = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "identity_game",
        StreamIdentityMode::Legacy,
        stream.len() as u64,
        &source_sha256,
        2,
    )
    .unwrap();
    assert_eq!(artifacts.campaign_identity_manifest, None);

    let dir = TmpDir::new("certified_identity_v1");
    let source = dir.0.join("collector_stream.bin");
    let stream = identity_v1_stream();
    std::fs::write(&source, &stream).unwrap();
    let source_sha256 = to_hex(&Sha256::digest(&stream));
    let artifact_root = dir.0.join("artifacts");
    std::fs::create_dir(&artifact_root).unwrap();

    let artifacts = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "identity_game",
        StreamIdentityMode::IdentityV1,
        stream.len() as u64,
        &source_sha256,
        2,
    )
    .unwrap();
    let retained = artifacts
        .campaign_identity_manifest
        .expect("IdentityV1 artifacts must retain the placement-authoritative CID1");
    assert_eq!(
        retained
            .get("game_unique_id")
            .and_then(serde_json::Value::as_str),
        Some(CAMPAIGN_UUID)
    );
}

fn domain_gap_payload(game_hour_raw: i32) -> Vec<u8> {
    compact_contract::DomainGapRecordV1 {
        part: compact_contract::DomainGapPart::Both,
        reason: compact_contract::DomainGapReason::ClockUnreadable,
        target_hour_raw: game_hour_raw,
        range_start_hour_raw: game_hour_raw,
        range_end_hour_raw: game_hour_raw,
        tick_before: 0,
        tick_after: 0,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    }
    .encode()
    .expect("canonical gap fixture")
}

fn settling_gap_hour_stream(gap_domains: &[u16]) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut seq = 1u64;
    for domain in gap_domains {
        push_record(
            &mut stream,
            seq,
            raw_for(HOUR_A),
            *domain,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            &domain_gap_payload(raw_for(HOUR_A)),
        );
        seq += 1;
    }
    push_record(
        &mut stream,
        seq,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload_with_perf_domains(gap_domains.len() as u32),
    );
    seq += 1;
    for hour in [HOUR_B, HOUR_C] {
        push_record(
            &mut stream,
            seq,
            raw_for(hour),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        seq += 1;
        push_record(
            &mut stream,
            seq,
            raw_for(hour),
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        seq += 1;
    }
    stream
}

#[test]
fn a_capture_opening_with_an_all_gap_hour_still_certifies() {
    let gap_domains: Vec<u16> = (10u16..=17).collect();
    let stream = settling_gap_hour_stream(&gap_domains);

    let dir = TmpDir::new("settling_gap_hour");
    let source = dir.0.join("collector_stream.bin");
    std::fs::write(&source, &stream).unwrap();
    let source_sha256 = to_hex(&Sha256::digest(&stream));
    let artifact_root = dir.0.join("artifacts");
    std::fs::create_dir(&artifact_root).unwrap();

    let artifacts = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "settling_game",
        StreamIdentityMode::Legacy,
        stream.len() as u64,
        &source_sha256,
        2,
    )
    .expect("a recording whose first hour is all gaps must still certify");

    assert_eq!(artifacts.hour_count, 3);
    let cache = open_certified_cache(&artifacts);
    assert_eq!(
        cache
            .index()
            .hours()
            .iter()
            .map(snapshot::ReplayHourEntry::hour_index)
            .collect::<Vec<_>>(),
        vec![HOUR_A, HOUR_B, HOUR_C]
    );

    let legacy_spill = dir.0.join("legacy_spill");
    std::fs::create_dir(&legacy_spill).unwrap();
    let legacy = legacy_packed_snapshot_series(&source, &legacy_spill, "settling_game");
    assert_eq!(legacy.len(), 3);
    for (hour_index, legacy_bytes) in &legacy {
        let (reconstructed, _) = cache.reconstruct_json(*hour_index).unwrap();
        assert_eq!(
            &reconstructed, legacy_bytes,
            "hour {hour_index} diverged from the legacy fold"
        );
    }
}

#[test]
fn terminal_outcomes_do_not_relax_hour_interleaving_or_a_bare_frame_end() {
    let dir = TmpDir::new("settling_gap_hour_bounds");

    let mut bare = Vec::new();
    push_record(
        &mut bare,
        1,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let source = dir.0.join("bare.bin");
    std::fs::write(&source, &bare).unwrap();
    let artifact_root = dir.0.join("bare_artifacts");
    std::fs::create_dir(&artifact_root).unwrap();
    let error = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "bare_game",
        StreamIdentityMode::Legacy,
        bare.len() as u64,
        &to_hex(&Sha256::digest(&bare)),
        2,
    )
    .expect_err("a FRAME_END with no terminal domain outcome must still be refused");
    assert!(
        format!("{error}").contains("without an open DOMAIN_DATA hour"),
        "the strict machine, not only the shared boundary tracker, must refuse this: {error}"
    );

    let mut interleaved = Vec::new();
    push_record(
        &mut interleaved,
        1,
        raw_for(HOUR_A),
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut interleaved,
        2,
        raw_for(HOUR_B),
        19,
        COMPACT_RECORD_TYPE_DOMAIN_GAP,
        &domain_gap_payload(raw_for(HOUR_B)),
    );
    push_record(
        &mut interleaved,
        3,
        raw_for(HOUR_A),
        DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let source = dir.0.join("interleaved.bin");
    std::fs::write(&source, &interleaved).unwrap();
    let artifact_root = dir.0.join("interleaved_artifacts");
    std::fs::create_dir(&artifact_root).unwrap();
    let error = build_certified_replay_artifacts(
        &source,
        &artifact_root,
        "interleaved_game",
        StreamIdentityMode::Legacy,
        interleaved.len() as u64,
        &to_hex(&Sha256::digest(&interleaved)),
        2,
    )
    .expect_err("a terminal outcome for another hour must still be interleaving");
    assert!(
        format!("{error}").contains("terminal domain outcome for hour"),
        "the strict machine, not only the shared boundary tracker, must refuse this: {error}"
    );
}

fn stream_with_an_absent_second_hour() -> Vec<u8> {
    let mut stream = Vec::new();
    let mut seq = 1u64;
    for hour in 0..4i64 {
        let raw = raw_for(HOUR_A + hour);
        let base = seq;
        seq += 2;
        if hour == 1 {
            continue;
        }
        push_record(
            &mut stream,
            base,
            raw,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut stream,
            base + 1,
            raw,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
    }
    stream
}

#[test]
fn an_interior_excluded_hour_becomes_a_named_hole_and_the_rest_of_the_recording_indexes() {
    let dir = TmpDir::new("gz_interior_hole");
    let path = dir.0.join("collector_stream.bin");
    let stream = stream_with_an_absent_second_hour();
    std::fs::write(&path, &stream).unwrap();

    let index = build_index(&Source::StreamFile(path), &OpenOptions::default())
        .expect("an interior hole must not cost the whole recording");

    let served: Vec<i64> = index.hours.iter().map(|hour| hour.hour_index).collect();
    let holes: Vec<i64> = index.excluded.iter().map(|hole| hole.hour_index).collect();
    assert_eq!(
        served,
        vec![HOUR_A, HOUR_A + 3],
        "every hour the recording vouches for is still served: served={served:?} holes={holes:?}"
    );
    assert_eq!(
        holes,
        vec![HOUR_A + 2],
        "the hour the stream resumed in is the DELIVERED excluded hour, so it is the one that \
         holds a place in the byte account; hour {} never arrived at all and occupies no bytes",
        HOUR_A + 1
    );
    for hole in &index.excluded {
        assert!(
            !hole.reason.trim().is_empty(),
            "an excluded hour is never guessed, only named: {hole:?}"
        );
        assert!(
            hole.end_offset > hole.start_offset,
            "the resuming hour's records are real bytes: {hole:?}"
        );
    }

    let ends = index.served_end_offsets();
    assert_eq!(ends.len(), 2);
    assert_eq!(
        ends[0], index.excluded[0].start_offset,
        "hour {HOUR_A}'s range must end where the excluded hour's records start"
    );
    assert!(
        ends[0] < index.hours[1].start_offset,
        "and must NOT reach over them to hour {}",
        HOUR_A + 3
    );
    assert_eq!(
        index.excluded[0].end_offset, index.hours[1].start_offset,
        "the hole's own range must abut the next served hour"
    );
    assert_eq!(ends[1], index.source.decoded_len);

    index
        .validate_structure()
        .expect("served hours and holes tile the source");
    assert_eq!(
        StreamIndex::from_json(&index.to_json()).as_ref(),
        Some(&index),
        "the holes survive the persisted form"
    );

    let source = Source::StreamFile(dir.0.join("collector_stream.bin"));
    let expected_reason = index.excluded[0].reason.clone();
    match seek_to_hour(&source, &index, HOUR_A + 2, &OpenOptions::default()) {
        Err(SeekError::HourExcluded { hour_index, reason }) => {
            assert_eq!(hour_index, HOUR_A + 2);
            assert_eq!(
                reason, expected_reason,
                "the refusal must carry the recording's own reason verbatim"
            );
        }
        other => panic!("an excluded hour must be refused by name, got {other:?}"),
    }
    match seek_to_hour(&source, &index, 999_999, &OpenOptions::default()) {
        Err(SeekError::HourNotFound(999_999)) => {}
        other => panic!("expected HourNotFound, got {other:?}"),
    }
    for served in [HOUR_A, HOUR_A + 3] {
        seek_to_hour(&source, &index, served, &OpenOptions::default())
            .unwrap_or_else(|error| panic!("verified hour {served} must still seek: {error}"));
    }

    let other_dir = TmpDir::new("gz_interior_hole_other_source");
    let other_path = other_dir.0.join("collector_stream.bin");
    std::fs::write(&other_path, three_hour_stream()).unwrap();
    match seek_to_hour(
        &Source::StreamFile(other_path),
        &index,
        HOUR_A + 2,
        &OpenOptions::default(),
    ) {
        Err(SeekError::InvalidIndex(_)) => {}
        other => panic!(
            "an index from another recording must not be able to name an hour excluded: {other:?}"
        ),
    }
}

#[test]
fn a_recording_whose_first_hour_is_excluded_claims_no_cheap_seek_point() {
    let dir = TmpDir::new("gz_leading_hole");
    let path = dir.0.join("collector_stream.bin");
    let mut stream = Vec::new();
    let mut seq = 1u64;
    for hour in 0..4i64 {
        let raw = raw_for(HOUR_A + hour);
        let base = seq;
        seq += 2;
        if hour == 0 {
            continue;
        }
        push_record(
            &mut stream,
            base,
            raw,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut stream,
            base + 1,
            raw,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
    }
    std::fs::write(&path, &stream).unwrap();

    let index = build_index(&Source::StreamFile(path), &OpenOptions::default())
        .expect("a leading hole must not cost the whole recording");
    assert_eq!(
        index.excluded.first().map(|hole| hole.start_offset),
        Some(0),
        "the excluded hour holds decoded offset zero: {:?}",
        index.excluded
    );
    assert!(
        index.hours[0].start_offset > 0,
        "the first SERVED hour starts after the hole"
    );
    assert!(
        index.cheap_seek_points().is_empty(),
        "no hour of this recording can be folded to from a fresh fold, so none may claim to be"
    );
    index
        .validate_structure()
        .expect("a leading hole still tiles the source");

    let mut forged = index.clone();
    forged.hours[0].cheap_seek_point = true;
    assert!(forged
        .validate_structure()
        .expect_err("a non-zero-offset cheap seek point must refuse")
        .contains("cheap seek point"));
}
