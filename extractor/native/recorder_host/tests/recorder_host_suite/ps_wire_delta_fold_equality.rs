//! The fold produces the SAME hours whether a delta-enabled domain's payload
//! rides whole or as a `PSD1` envelope (lane PS).
//!
//! This is the in-repo half of the equality proof. The other half runs offline
//! over the sealed spools — `extractor/native/runs/ps_wire_delta/`, where
//! `stream_fold` over `captures/oz_tip` and over the same capture re-encoded
//! through this codec produce the identical snapshot chain hash — but that
//! needs a multi-gigabyte capture and cannot live in the test suite. This does
//! the same comparison on a stream the test builds.
//!
//! # Why the payloads are not valid domain grammars, and why that is the point
//!
//! The fold's answer to a d41/d43/d53 payload it cannot parse is a NAMED
//! malformed-payload outcome carrying the parser's own complaint, and that
//! complaint is a function of the payload BYTES. So two streams whose records
//! reconstruct to the same bytes must produce the same complaint, in the same
//! hour, for the same domain — and a fold that failed to unwrap would complain
//! about envelope bytes instead and produce visibly different hours. That is
//! exactly the failure this test was written against: before the driving fold
//! loop reconstructed, `captures/pn_structural` re-encoded through the codec
//! folded 519 records into `malformed_payload_domains` with reasons like
//! "diplomacy/intel schema_version 72 != 2" — the parser reading a `PSD1`
//! header as a d53 header.
//!
//! Building genuinely valid d41/d43/d53 payloads here would need three live
//! extractors; it would also prove something weaker, because a valid payload
//! folds into state whose equality is checked by the same snapshot comparison
//! this already makes.

use compact_contract::payload_span_delta as psd;
use compact_contract::{
    COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
};
use recorder_host::world_fold::WorldFold;

const FRAME_DOMAIN: u16 = 1000;
const FIRST_HOUR: i32 = 60_830_824;

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

/// One hour's payload for a domain: `elapsed_wall_ms` at offset 0, then a body
/// that mostly repeats the previous hour's and moves in the three ways the
/// sealed captures show — a few bytes in place, an insertion, a truncation.
fn domain_payload(domain_id: u16, hour: u64, body: &[u8]) -> Vec<u8> {
    let mut payload = (hour * 977 + u64::from(domain_id)).to_le_bytes().to_vec();
    payload.extend_from_slice(body);
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

struct BuiltStreams {
    raw: Vec<u8>,
    wrapped: Vec<u8>,
    raw_payload_bytes: usize,
    wrapped_payload_bytes: usize,
}

fn build_streams(hours: u64) -> BuiltStreams {
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

    for hour in 0..hours {
        let game_hour_raw = FIRST_HOUR + hour as i32;
        for (domain_id, body) in bodies.iter_mut() {
            evolve(body, hour);
            let payload = domain_payload(*domain_id, hour, body);
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
                    true,
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
    BuiltStreams {
        raw,
        wrapped,
        raw_payload_bytes,
        wrapped_payload_bytes,
    }
}

fn fold_hours(stream: &[u8]) -> Vec<(i64, String, Vec<u8>)> {
    let mut fold = WorldFold::new("ps_wire_delta_equality");
    fold.ingest_stream(stream);
    fold.finish()
}

#[test]
fn a_wrapped_stream_folds_to_exactly_the_same_hours_as_the_raw_one() {
    let built = build_streams(96);
    let raw_hours = fold_hours(&built.raw);
    let wrapped_hours = fold_hours(&built.wrapped);

    assert_eq!(raw_hours.len(), 96, "the raw stream must publish every hour");
    // kills: any failure to unwrap in the fold. An enveloped record folded as if
    // it were the domain's own grammar produces a DIFFERENT hour -- a different
    // malformed-payload reason, a different provenance -- and the snapshots stop
    // matching. This is the exact regression the driving fold loop had before it
    // learned to reconstruct.
    assert_eq!(
        raw_hours.len(),
        wrapped_hours.len(),
        "hour counts differ: {} vs {}",
        raw_hours.len(),
        wrapped_hours.len()
    );
    for (raw, delta) in raw_hours.iter().zip(wrapped_hours.iter()) {
        assert_eq!(raw.0, delta.0, "hour index");
        assert_eq!(raw.1, delta.1, "hour date");
        assert_eq!(
            raw.2,
            delta.2,
            "hour {} snapshot differs between the raw and the enveloped stream",
            raw.1
        );
    }

    // kills: a codec that is byte-identical but buys nothing. The bodies above
    // change by tens of bytes an hour against 8 KiB payloads; if the wrapped
    // stream is not far smaller, the delta is not deltaing.
    assert!(
        built.wrapped_payload_bytes * 8 < built.raw_payload_bytes,
        "wrapped {} vs raw {} payload bytes",
        built.wrapped_payload_bytes,
        built.raw_payload_bytes
    );
}

#[test]
fn a_stale_consumer_baseline_refuses_the_hour_by_name_and_the_cadence_recovers_it() {
    // kills: a fold that applies a delta to whatever baseline it happens to hold.
    // The stream below has one record REMOVED, so every later delta names a
    // baseline this fold cannot produce until the next keyframe.
    let built = build_streams((psd::KEYFRAME_CADENCE_HOURS as u64) + 8);
    let mut truncated = Vec::with_capacity(built.wrapped.len());
    let mut cursor = 0usize;
    let mut dropped = 0usize;
    while cursor < built.wrapped.len() {
        let header = compact_contract::parse_compact_record_header(&built.wrapped[cursor..])
            .expect("framed");
        let record_len = header.record_len();
        let drop_this = dropped == 0
            && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            && header.game_hour_raw == FIRST_HOUR + 2;
        if drop_this {
            dropped += 1;
        } else {
            truncated.extend_from_slice(&built.wrapped[cursor..cursor + record_len]);
        }
        cursor += record_len;
    }
    assert_eq!(dropped, 1);

    let hours = fold_hours(&truncated);
    // Every hour still SEALS -- partial data still seals, and a refusal excludes
    // and names rather than destroying the hour.
    assert_eq!(hours.len(), (psd::KEYFRAME_CADENCE_HOURS as usize) + 8);
    let refusals = hours
        .iter()
        .filter(|(_, _, bytes)| {
            String::from_utf8_lossy(bytes).contains("PSD1 retained baseline")
        })
        .count();
    assert!(
        refusals > 0,
        "a stale baseline must produce NAMED refusals, not a silent misapply"
    );
    // kills: deleting the keyframe cadence. Without it the refusals run to the
    // end of the capture; with it the domain re-synchronises.
    assert!(
        refusals < hours.len(),
        "the keyframe cadence must resynchronise the fold: {refusals} of {} hours refused",
        hours.len()
    );
}
