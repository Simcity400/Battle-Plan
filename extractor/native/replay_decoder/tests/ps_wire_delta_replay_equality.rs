//! The CERTIFIED REPLAY PRODUCT is the same whether a delta-enabled domain's
//! payload rides whole or as a `PSD1` envelope (lane PS, Codex review round 7).
//!
//! # Why this test exists and the fold-level one was not enough
//!
//! `recorder_host`'s `ps_wire_delta_fold_equality` drives `WorldFold` directly,
//! and `recorder_host::stream_fold`'s driving loop learned to reconstruct early.
//! `replay_decoder` is a DIFFERENT WALKER of the same stream — two of them, in
//! fact: the spooled fold and [`replay_decoder::HourCursor`] — and it is the one
//! that produces the certified artifacts. Both validated the envelope bytes as
//! the domain's own grammar, marked every wrapped d43/d53 record malformed, and
//! SKIPPED the `ingest_record` call that would have reconstructed it. Every
//! wrapped record was excluded from the certified product while the
//! `stream_fold` artifacts looked clean, because those are not this code.
//!
//! So the property is asserted here, against this crate's own public entry
//! point, rather than inferred from a sibling crate's.
//!
//! # Why the payloads are not valid domain grammars
//!
//! Same reason as the fold-level test: the decoder's answer to a payload it
//! cannot parse is a NAMED outcome carrying the parser's own complaint, and that
//! complaint is a function of the payload BYTES. Two streams whose records
//! reconstruct to the same bytes must therefore produce the same hours — and a
//! walker that failed to unwrap complains about envelope bytes instead, which is
//! exactly the defect above. Building genuinely valid d43/d53 payloads would need
//! two live extractors and would prove something weaker.

use std::io::Write;

use compact_contract::payload_span_delta as psd;
use compact_contract::{
    COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
};

const FRAME_DOMAIN: u16 = 1000;
const FIRST_HOUR: i32 = 60_830_824;
const HOURS: u64 = 24;

fn append_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    flags: u32,
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
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(payload);
}

/// A minimal DOMAIN_PERF payload: the terminal outcome every scheduled domain
/// owes its frame.
///
/// The frame seal counts unique non-probe terminal outcomes (a DOMAIN_PERF or a
/// DOMAIN_GAP per domain) against what FRAME_END claims it emitted, and a frame
/// that claims two and shows none is BOUNDARY DAMAGE -- which the decoder reports
/// in the hour as free text naming a byte position, and byte positions differ
/// between a stream and the same stream re-encoded smaller. Emitting the perf
/// record the contract requires removes that whole class of noise from this
/// comparison, and leaves the payload rejection, which is the signal this test
/// actually reads.
fn perf_payload(elapsed_wall_ms: u64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());
    for _ in 0..4 {
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    debug_assert_eq!(payload.len(), 40);
    payload
}

fn frame_end_payload(elapsed_wall_ms: u64, emitted_domains: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&1_000u64.to_le_bytes());
    payload.extend_from_slice(&emitted_domains.to_le_bytes());
    payload.extend_from_slice(&emitted_domains.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn evolve(body: &mut Vec<u8>, hour: u64) {
    match hour % 6 {
        1 => {
            let at = (hour as usize * 131) % (body.len() - 1);
            body[at] ^= 0x5a;
        }
        2 => {
            let inserted: Vec<u8> = (0..40u8).map(|b| b ^ hour as u8).collect();
            body.splice(700..700, inserted);
        }
        4 => {
            body.truncate(body.len() - 16);
        }
        _ => {}
    }
}

struct Streams {
    raw: Vec<u8>,
    wrapped: Vec<u8>,
    raw_payload_bytes: usize,
    wrapped_payload_bytes: usize,
}

fn build_streams() -> Streams {
    let mut producer = psd::PayloadSpanDeltaProducer::new();
    let mut raw = Vec::new();
    let mut wrapped = Vec::new();
    let mut raw_payload_bytes = 0usize;
    let mut wrapped_payload_bytes = 0usize;
    let mut sequence = 1u64;
    let mut bodies: Vec<(u16, Vec<u8>)> = psd::DELTA_ENABLED_DOMAINS
        .iter()
        .enumerate()
        .map(|(index, domain_id)| {
            (
                *domain_id,
                (0..8192usize)
                    .map(|i| (i as u8).wrapping_mul(37).wrapping_add(index as u8))
                    .collect(),
            )
        })
        .collect();

    for hour in 0..HOURS {
        let game_hour_raw = FIRST_HOUR + hour as i32;
        for (domain_id, body) in bodies.iter_mut() {
            evolve(body, hour);
            let perf = perf_payload(hour * 1_000);
            for stream in [&mut raw, &mut wrapped] {
                append_record(
                    stream,
                    sequence,
                    game_hour_raw,
                    *domain_id,
                    compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    0,
                    &perf,
                );
            }
            sequence += 1;
            let mut payload = (hour * 977 + u64::from(*domain_id)).to_le_bytes().to_vec();
            payload.extend_from_slice(body);
            raw_payload_bytes += payload.len();
            append_record(
                &mut raw,
                sequence,
                game_hour_raw,
                *domain_id,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
            let envelope = producer
                .wrap(
                    psd::PayloadSpanDeltaIdentity {
                        domain_id: *domain_id,
                        source_sequence: sequence,
                        game_hour_raw,
                    },
                    &payload,
                )
                .expect("wrap");
            wrapped_payload_bytes += envelope.len();
            append_record(
                &mut wrapped,
                sequence,
                game_hour_raw,
                *domain_id,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                &envelope,
            );
            sequence += 1;
        }
        producer.commit();
        let end = frame_end_payload(hour * 1_000, bodies.len() as u32);
        for stream in [&mut raw, &mut wrapped] {
            append_record(
                stream,
                sequence,
                game_hour_raw,
                FRAME_DOMAIN,
                COMPACT_RECORD_TYPE_FRAME_END,
                0,
                &end,
            );
        }
        sequence += 1;
    }
    Streams {
        raw,
        wrapped,
        raw_payload_bytes,
        wrapped_payload_bytes,
    }
}

fn write_stream(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("create stream file");
    file.write_all(bytes).expect("write stream file");
    file.flush().expect("flush stream file");
    path
}

/// Rewrite every `offset NNNN:` to `offset N:`, and count how many there were.
///
/// # Why the comparison needs this, and why it still kills the defect
///
/// A payload this decoder cannot parse is reported in its hour as free text
/// naming the record's BYTE POSITION IN THE STREAM. The enveloped stream is
/// smaller, so the same rejection of the same record cites a different offset --
/// a statement about the spool that was decoded, not a folded value, and one no
/// size change can avoid. It is the same citation `normalised_snapshot_hashes.py`
/// normalises away for `captures/oz_tip` in
/// `extractor/native/runs/ps_wire_delta/`.
///
/// What survives normalisation is everything the defect this test exists for
/// would change: a walker that failed to unwrap parses ENVELOPE bytes as d43/d53
/// grammar and complains about different schema versions, scales and row counts,
/// none of which sit inside an offset citation. The CITATION COUNT is asserted
/// separately, so a record that vanished from one stream is still caught.
fn normalise_offsets(snapshot: &[u8]) -> (Vec<u8>, usize) {
    // `offset ` and not `(offset `: only the FIRST citation in a rejection
    // sentence is parenthesised, the rest are separated by `; `.
    const NEEDLE: &[u8] = b"offset ";
    let mut out = Vec::with_capacity(snapshot.len());
    let mut citations = 0usize;
    let mut at = 0usize;
    while at < snapshot.len() {
        if snapshot[at..].starts_with(NEEDLE) {
            let digits_start = at + NEEDLE.len();
            let mut end = digits_start;
            while end < snapshot.len() && snapshot[end].is_ascii_digit() {
                end += 1;
            }
            if end > digits_start && snapshot.get(end) == Some(&b':') {
                out.extend_from_slice(NEEDLE);
                out.push(b'N');
                citations += 1;
                at = end;
                continue;
            }
        }
        out.push(snapshot[at]);
        at += 1;
    }
    (out, citations)
}

/// Every hour the cursor publishes, as (index, date, snapshot bytes).
fn cursor_hours(path: &std::path::Path) -> Vec<(i64, String, Vec<u8>)> {
    let opts = replay_decoder::OpenOptions {
        game_id: Some("ps_wire_delta_replay_equality".to_string()),
    };
    let mut cursor = replay_decoder::open_stream_file(path, &opts).expect("open stream file");
    let mut hours = Vec::new();
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.expect("decode hour");
        hours.push((hour.hour_index, hour.date.clone(), hour.snapshot.clone()));
    }
    hours
}

#[test]
fn the_replay_cursor_publishes_identical_hours_for_a_wrapped_stream() {
    let built = build_streams();
    let dir = std::env::temp_dir().join(format!(
        "ps_wire_delta_replay_equality_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let raw_path = write_stream(&dir, "raw.bin", &built.raw);
    let wrapped_path = write_stream(&dir, "wrapped.bin", &built.wrapped);

    let raw_hours = cursor_hours(&raw_path);
    let wrapped_hours = cursor_hours(&wrapped_path);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        raw_hours.len(),
        HOURS as usize,
        "the raw stream must publish every hour"
    );
    // kills: validating an envelope as the domain's own grammar in
    // `replay_decoder`. An un-reconstructed record is marked malformed and never
    // reaches `ingest_record`, so its hour carries a different complaint and the
    // snapshots stop matching -- which is exactly what shipped until Codex round
    // 7 found it, with `stream_fold`'s artifacts clean the whole time.
    assert_eq!(
        raw_hours.len(),
        wrapped_hours.len(),
        "hour counts differ: {} vs {}",
        raw_hours.len(),
        wrapped_hours.len()
    );
    let mut citations_seen = 0usize;
    for (raw, delta) in raw_hours.iter().zip(wrapped_hours.iter()) {
        assert_eq!(raw.0, delta.0, "hour index");
        assert_eq!(raw.1, delta.1, "hour date");
        let (raw_norm, raw_citations) = normalise_offsets(&raw.2);
        let (delta_norm, delta_citations) = normalise_offsets(&delta.2);
        // kills: a record that reached one walker and not the other. The offsets
        // are normalised; the COUNT of them is not.
        assert_eq!(
            raw_citations, delta_citations,
            "hour {} cites {raw_citations} stream offsets raw and {delta_citations} enveloped",
            raw.1
        );
        citations_seen += raw_citations;
        if raw_norm != delta_norm {
            let a = String::from_utf8_lossy(&raw_norm);
            let b = String::from_utf8_lossy(&delta_norm);
            let at = a
                .as_bytes()
                .iter()
                .zip(b.as_bytes())
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(b.len()));
            let lo = at.saturating_sub(160);
            panic!(
                "hour {} snapshot differs at byte {at} (lengths {} vs {}); raw [{}] enveloped [{}]",
                raw.1,
                raw_norm.len(),
                delta_norm.len(),
                &a[lo..(at + 160).min(a.len())],
                &b[lo..(at + 160).min(b.len())]
            );
        }
    }
    // kills: a normalisation that silently matched everything because the hours
    // carry no rejection text at all -- in which case this test would be passing
    // on empty hours rather than on equal ones.
    assert!(
        citations_seen > 0,
        "these payloads are not valid d43/d53 grammar, so every hour must carry a named rejection"
    );

    // kills: a codec that is byte-identical but buys nothing. The bodies change
    // by tens of bytes an hour against 8 KiB payloads.
    assert!(
        built.wrapped_payload_bytes * 4 < built.raw_payload_bytes,
        "wrapped {} vs raw {} payload bytes",
        built.wrapped_payload_bytes,
        built.raw_payload_bytes
    );
}
