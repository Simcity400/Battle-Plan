//! The `PSD1` producer transaction, driven through the COLLECTOR's own wiring.
//!
//! The codec's unit tests call `commit` and `roll_back` directly, which proves
//! the transaction's own logic and nothing about whether this crate calls it.
//! These drive the real emission helper
//! ([`append_compact_span_delta_data_record`]), the real per-domain rollback in
//! [`reset_trackers_after_rejected_bracket`], and the real seal in
//! [`commit_delta_pin_transactions`], against a `CompactPerfGateTrackers` that
//! PERSISTS across frames — so a regression in any of the three shows up here
//! rather than in a capture. (Codex review round 1, finding 4.)
//!
//! The consumer is the shipped [`PayloadSpanDeltaConsumer`], fed only the records
//! a frame actually published, which is exactly what the fold sees.

use super::*;
use compact_contract::payload_span_delta as psd;

const D53: u16 = COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE;
const D43: u16 = COMPACT_DOMAIN_MARKET_TRADE_LIVE;
const FIRST_HOUR: i32 = 60_830_824;
/// A d53-shaped payload: `elapsed_wall_ms` at offset 0, then a body.
fn payload(elapsed_wall_ms: u64, body: &[u8]) -> Vec<u8> {
    let mut out = elapsed_wall_ms.to_le_bytes().to_vec();
    out.extend_from_slice(body);
    out
}

fn body_of_len(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(17).wrapping_add(seed))
        .collect()
}

/// Emit one DOMAIN_DATA record through the shipped helper and return the record
/// bytes exactly as the frame would carry them.
fn emit(
    trackers: &mut CompactPerfGateTrackers,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    payload: &[u8],
) -> Vec<u8> {
    emit_with_status(trackers, sequence, game_hour_raw, domain_id, 0, payload)
}

fn emit_with_status(
    trackers: &mut CompactPerfGateTrackers,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    status_flags: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    append_compact_span_delta_data_record(
        &mut out,
        &mut trackers.payload_span_delta,
        sequence,
        game_hour_raw,
        domain_id,
        status_flags,
        payload,
    );
    out
}

struct Published {
    identity: psd::PayloadSpanDeltaIdentity,
    record_flags: u32,
    payload: Vec<u8>,
    wire_len: usize,
}

fn parse_one(record: &[u8]) -> Published {
    let header =
        compact_contract::parse_compact_record_header(record).expect("well-framed H4CR record");
    assert_eq!(header.record_len(), record.len(), "exactly one record");
    assert_eq!(header.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);
    compact_contract::record_semantics::validate_compact_record_semantics(
        header.domain_id,
        header.record_type,
        header.record_flags,
        &record[COMPACT_RECORD_HEADER_LEN as usize..],
    )
    .expect("the shipped record grammar admits the flag this helper sets");
    Published {
        identity: psd::PayloadSpanDeltaIdentity {
            domain_id: header.domain_id,
            source_sequence: header.sequence,
            game_hour_raw: header.game_hour_raw,
        },
        record_flags: header.record_flags,
        payload: record[COMPACT_RECORD_HEADER_LEN as usize..].to_vec(),
        wire_len: header.payload_len,
    }
}

fn settle(trackers: &mut CompactPerfGateTrackers, rejected: &[u16]) {
    let rejected: BTreeSet<u16> = rejected.iter().copied().collect();
    if !rejected.is_empty() {
        reset_trackers_after_rejected_bracket(trackers, &rejected);
    }
    commit_delta_pin_transactions(trackers, &mut Vec::new());
}

fn apply(consumer: &mut psd::PayloadSpanDeltaConsumer, published: &Published) -> Vec<u8> {
    psd::reconstruct_if_enveloped(
        consumer,
        published.record_flags,
        published.identity,
        &published.payload,
    )
    .expect("the fold reconstructs a published record")
    .expect("a delta-enabled domain's record is an envelope")
}

#[test]
fn a_rejected_frame_leaves_the_collectors_producer_where_the_fold_is() {
    // kills: deleting the per-domain `roll_back` in
    // `reset_trackers_after_rejected_bracket`, or the `commit` in
    // `commit_delta_pin_transactions`. The middle frame's record becomes a
    // DOMAIN_GAP and the fold never receives it, so a producer that kept the
    // advance would name a baseline the fold cannot produce and the THIRD frame
    // would refuse by name -- and every frame after it, to the end of the
    // capture.
    let mut trackers = CompactPerfGateTrackers::default();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    let mut body = body_of_len(65_536, 3);

    let first = payload(1_000, &body);
    let record = emit(&mut trackers, 1, FIRST_HOUR, D53, &first);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_eq!(apply(&mut consumer, &published), first);

    // This frame's d53 record is replaced by a DOMAIN_GAP: it is emitted, then
    // rejected, and the consumer below is never shown it.
    body[10_000] ^= 0xff;
    let dropped = payload(2_000, &body);
    let _ = emit(&mut trackers, 2, FIRST_HOUR + 1, D53, &dropped);
    settle(&mut trackers, &[D53]);

    body[20_000] ^= 0xff;
    let third = payload(3_000, &body);
    let record = emit(&mut trackers, 3, FIRST_HOUR + 2, D53, &third);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_eq!(apply(&mut consumer, &published), third);
    // kills: deleting `trackers.payload_span_delta.commit()` from
    // `commit_delta_pin_transactions` (Codex review round 2, finding 2).
    // Equality alone cannot see that: with no commit, `current()` keeps reading
    // the STAGED baseline, the rollback above clears it, and the third frame
    // safely falls back to a keyframe -- correct, and 64 KiB instead of a few
    // dozen bytes, every time any bracket is rejected. Only the SIZE says which
    // of the two happened.
    assert!(
        published.wire_len < 1024,
        "the frame after a rejection re-sent {} bytes; the committed baseline was lost",
        published.wire_len
    );
}

#[test]
fn a_frames_records_survive_the_drop_passes_renumbering_them() {
    // kills: deleting the `PSD1` arm of `stamp_record_bound_sequence`, or
    // wrapping a domain that bakes its own sequence (Codex review round 2,
    // finding 1). The producer wraps during the BUILD, and the frame's drop
    // passes then remove records and `resequence_compact_records` renumbers every
    // survivor by position -- so an envelope's baked sequence overshoots by the
    // number of records dropped ahead of it, and the consumer's identity join
    // refuses the hour for arithmetic rather than for anything the game did.
    //
    // This drives the real resequencer over a real frame, exactly as the emission
    // path does after a rejected domain's records became gaps.
    let mut trackers = CompactPerfGateTrackers::default();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    let mut body = body_of_len(65_536, 11);

    let mut frame = Vec::new();
    append_compact_span_delta_data_record(
        &mut frame,
        &mut trackers.payload_span_delta,
        9,
        FIRST_HOUR,
        D53,
        0,
        &payload(1_000, &body),
    );
    settle(&mut trackers, &[]);
    // Two records ahead of it were dropped: the stream ships this one as 7.
    resequence_compact_records(&mut frame, 7).expect("the frame resequences");
    let published = parse_one(&frame);
    assert_eq!(published.identity.source_sequence, 7);
    assert_eq!(apply(&mut consumer, &published), payload(1_000, &body));

    // and the same again for a DELTA body, whose header is the only place the
    // sequence lives.
    body[4_000] ^= 0xff;
    let second = payload(2_000, &body);
    let mut frame = Vec::new();
    append_compact_span_delta_data_record(
        &mut frame,
        &mut trackers.payload_span_delta,
        14,
        FIRST_HOUR + 1,
        D53,
        0,
        &second,
    );
    settle(&mut trackers, &[]);
    resequence_compact_records(&mut frame, 8).expect("the frame resequences");
    let published = parse_one(&frame);
    assert_eq!(published.identity.source_sequence, 8);
    assert!(published.wire_len < 1024, "this hour must ride as a delta");
    assert_eq!(apply(&mut consumer, &published), second);
}

#[test]
fn every_domain_that_bakes_its_own_sequence_is_kept_out_of_the_enabled_set() {
    // kills: enabling a domain whose payload already carries a `record_sequence`
    // that `stamp_record_bound_sequence` settles by walking the payload (Codex
    // review round 2, finding 1). d41's PPA1 trailer is the case that forced this:
    // it is appended LAST, so inside an envelope its bytes are the last bytes of
    // the digested basis, and the stamper would rewrite eight of them underneath
    // `basis_sha256`.
    for domain_id in [
        COMPACT_DOMAIN_PROJECTS,
        COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE,
        COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE,
        COMPACT_DOMAIN_MAP_FURNITURE_VICTORY_POINTS_LIVE,
    ] {
        assert!(
            !psd::is_delta_enabled_domain(domain_id),
            "domain {domain_id} bakes its own record sequence and must not be wrapped"
        );
    }
}

#[test]
fn a_surviving_domain_keeps_its_advance_when_a_sibling_is_rejected() {
    // kills: rolling the whole store back instead of the rejected domain. d43
    // published its record, the fold applied it, and rewinding d43's baseline
    // because d53 failed would leave the producer BEHIND the fold -- which the
    // digest turns into a refused hour the next time d43 changes.
    let mut trackers = CompactPerfGateTrackers::default();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    let mut d43_body = body_of_len(32_768, 9);

    let first = payload(1_000, &d43_body);
    let record = emit(&mut trackers, 1, FIRST_HOUR, D43, &first);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_eq!(apply(&mut consumer, &published), first);

    d43_body[500] ^= 0xff;
    let second = payload(2_000, &d43_body);
    let record = emit(&mut trackers, 2, FIRST_HOUR + 1, D43, &second);
    // d53's bracket was rejected this frame; d43's was not.
    settle(&mut trackers, &[D53]);
    let published = parse_one(&record);
    assert_eq!(apply(&mut consumer, &published), second);

    d43_body[600] ^= 0xff;
    let third = payload(3_000, &d43_body);
    let record = emit(&mut trackers, 3, FIRST_HOUR + 2, D43, &third);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_eq!(apply(&mut consumer, &published), third);
}

#[test]
fn a_second_record_in_the_same_frame_deltas_against_the_first() {
    // kills: diffing every record of a frame against the COMMITTED baseline
    // instead of the STAGED one. The consumer applies records in stream order, so
    // a producer that ignored the staged baseline would disagree with it about
    // what the second record of a frame is a delta against, and every later hour
    // of that domain would refuse until the cadence.
    //
    // NOT a live path today, and an earlier wording here was wrong to imply it
    // was. d53 really does emit more than one DOMAIN_DATA record in a frame --
    // the A30 companion rows -- but those are appended RAW and UNFLAGGED by
    // `append_compact_diplomacy_intel_operatives_data_record`, and
    // `append_compact_span_delta_data_record` has one call site per enabled
    // domain, each reached once a frame (three since lane QL enabled d47). This
    // guards the invariant that makes rollback correct, at the collector's own
    // wiring rather than in the codec's unit tests.
    let mut trackers = CompactPerfGateTrackers::default();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    let mut body = body_of_len(65_536, 5);

    let first = payload(1_000, &body);
    let record = emit(&mut trackers, 1, FIRST_HOUR, D53, &first);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_eq!(apply(&mut consumer, &published), first);

    body[100] ^= 0xff;
    let second = payload(2_000, &body);
    body[200] ^= 0xff;
    let third = payload(2_000, &body);
    let second_record = emit(&mut trackers, 2, FIRST_HOUR + 1, D53, &second);
    let third_record = emit(&mut trackers, 3, FIRST_HOUR + 1, D53, &third);
    settle(&mut trackers, &[]);

    let second_published = parse_one(&second_record);
    let third_published = parse_one(&third_record);
    assert_eq!(apply(&mut consumer, &second_published), second);
    assert_eq!(apply(&mut consumer, &third_published), third);
    // kills: the second record falling back to a keyframe. It differs from the
    // first by one byte, so it must ride as a delta of a few dozen bytes rather
    // than re-sending 64 KiB.
    assert!(
        third_published.wire_len < 1024,
        "the second record of the frame cost {} bytes",
        third_published.wire_len
    );
}

#[test]
fn a_frames_records_carry_the_flag_and_this_hours_clock() {
    // kills: emitting the envelope without the record flag (no consumer would
    // ever unwrap it), and carrying the BASELINE's `elapsed_wall_ms` rather than
    // this hour's. The second record below is a no-change body -- identical
    // engine content, a different frame clock -- which is the one case where a
    // reconstruction that took the clock from the baseline would go unnoticed by
    // every digest in the format.
    let mut trackers = CompactPerfGateTrackers::default();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    let body = body_of_len(65_536, 7);

    let first = payload(1_000, &body);
    let record = emit(&mut trackers, 1, FIRST_HOUR, D53, &first);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_ne!(
        published.record_flags & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
        0,
        "the emitted record must declare that its payload is an envelope"
    );
    assert_eq!(apply(&mut consumer, &published), first);

    let unchanged_content_new_clock = payload(2_000, &body);
    let record = emit(&mut trackers, 2, FIRST_HOUR + 1, D53, &unchanged_content_new_clock);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_eq!(
        published.wire_len,
        psd::HEADER_LEN,
        "an unchanged hour must be a header and nothing else"
    );
    let applied = apply(&mut consumer, &published);
    assert_eq!(applied, unchanged_content_new_clock);
    assert_ne!(applied, first);
}

#[test]
fn a_payload_the_producer_cannot_wrap_still_rides_whole_and_unflagged() {
    // kills: turning a producer refusal into a lost or truncated record. The
    // payload below is shorter than the declared volatile header, so no basis
    // can be taken from it; the hour must keep every byte it had, exactly as it
    // did before this codec existed.
    let mut trackers = CompactPerfGateTrackers::default();
    let short = vec![1u8, 2, 3];
    let record = emit(&mut trackers, 1, FIRST_HOUR, D53, &short);
    let published = parse_one(&record);
    assert_eq!(
        published.record_flags & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
        0
    );
    assert_eq!(published.payload, short);

    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    assert_eq!(
        psd::reconstruct_if_enveloped(
            &mut consumer,
            published.record_flags,
            published.identity,
            &published.payload,
        )
        .unwrap(),
        None,
        "an unflagged record is passed through untouched"
    );
}

#[test]
fn d47s_emission_site_wraps_through_the_real_appender_and_the_consumer_reconstructs_it() {
    // kills: reverting `append_compact_characters_leaders_cabinet_data_record`
    // to a plain `append_compact_record` while 47 stays on
    // `DELTA_ENABLED_DOMAINS`. That combination is silent — every record rides
    // unflagged, consumers pass it through untouched, nothing refuses, and the
    // domain's measured 45x simply stops existing — and unlike a codec refusal
    // there is no error anywhere to catch it. This drives the REAL per-domain
    // emission function with the REAL tracker set across three frames: the first
    // hour must carry the envelope flag and reconstruct byte-for-byte through
    // the shipped `PayloadSpanDeltaConsumer` (the same reconstruction every
    // walker performs); a one-byte change must ride as a small delta rather
    // than re-send the payload; and an UNCHANGED-content third hour (only the
    // recorder clock moved) must cost EXACTLY `psd::HEADER_LEN` bytes — the one
    // observable that separates a no-change envelope from appended raw bytes,
    // since raw appending would re-send all 32 KiB.
    let mut trackers = CompactPerfGateTrackers::default();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();

    let mut body = body_of_len(32_768, 47);
    // A plausible d47 shape: the declared volatile header, then grammar bytes.
    // The appender is grammar-agnostic — its basis is payload[8..] whatever the
    // domain says — so a synthetic body exercises exactly the wiring under test.
    let frame_payload = |elapsed_wall_ms: u64, body: &[u8]| -> Vec<u8> {
        let mut out = elapsed_wall_ms.to_le_bytes().to_vec();
        out.extend_from_slice(body);
        out
    };
    let read_for = |payload_bytes: Vec<u8>| domain47::scheduled::NativeScheduledRead {
        payload: payload_bytes,
        ..Default::default()
    };

    let first = frame_payload(1_000, &body);
    let mut out = Vec::new();
    append_compact_characters_leaders_cabinet_data_record(
        &mut out,
        &mut trackers.payload_span_delta,
        1,
        FIRST_HOUR,
        &read_for(first.clone()),
        None,
    );
    settle(&mut trackers, &[]);
    let published = parse_one(&out);
    assert_ne!(
        published.record_flags & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
        0,
        "d47's emission site must declare its payload an envelope"
    );
    assert_eq!(apply(&mut consumer, &published), first);

    body[500] ^= 0xff;
    let second = frame_payload(2_000, &body);
    let mut out = Vec::new();
    append_compact_characters_leaders_cabinet_data_record(
        &mut out,
        &mut trackers.payload_span_delta,
        2,
        FIRST_HOUR + 1,
        &read_for(second.clone()),
        None,
    );
    settle(&mut trackers, &[]);
    let published = parse_one(&out);
    assert!(
        published.wire_len < 1024,
        "a one-byte change must ride as a delta against the committed baseline, \
         not re-send {} bytes",
        published.wire_len
    );
    assert_eq!(apply(&mut consumer, &published), second);

    let unchanged = frame_payload(3_000, &body);
    let mut out = Vec::new();
    append_compact_characters_leaders_cabinet_data_record(
        &mut out,
        &mut trackers.payload_span_delta,
        3,
        FIRST_HOUR + 2,
        &read_for(unchanged.clone()),
        None,
    );
    settle(&mut trackers, &[]);
    let published = parse_one(&out);
    assert_eq!(
        published.wire_len,
        psd::HEADER_LEN,
        "an unchanged hour through the real appender must be a header and nothing else"
    );
    assert_eq!(apply(&mut consumer, &published), unchanged);
}

#[test]
fn a_failure_status_record_retains_the_basis_on_both_sides_of_the_collectors_wiring() {
    // kills: passing `true` for `advances_basis` in
    // `append_compact_span_delta_data_record`. Capture 07's d53 stream
    // alternated full payloads with 72-byte source-unavailable records; a
    // producer that made the 72-byte payload its basis sent every following
    // full payload as 6.7 MB of literals.
    let mut trackers = CompactPerfGateTrackers::default();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    let mut body = body_of_len(65_536, 5);

    let full = payload(1_000, &body);
    let record = emit(&mut trackers, 1, FIRST_HOUR, D53, &full);
    settle(&mut trackers, &[]);
    assert_eq!(apply(&mut consumer, &parse_one(&record)), full);

    let failure = payload(2_000, &[0u8; 64]);
    let record = emit_with_status(
        &mut trackers,
        2,
        FIRST_HOUR + 1,
        D53,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        &failure,
    );
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert_eq!(
        published.payload[psd::FLAGS_OFFSET],
        psd::FLAG_RETAINS_BASIS,
        "a failure record declares that it leaves the basis where it found it"
    );
    assert_eq!(apply(&mut consumer, &published), failure);

    body[30_000] ^= 0xff;
    let third = payload(3_000, &body);
    let record = emit(&mut trackers, 3, FIRST_HOUR + 2, D53, &third);
    settle(&mut trackers, &[]);
    let published = parse_one(&record);
    assert!(
        published.wire_len < 1024,
        "the hour after a failure record re-sent {} bytes: the 72-byte record became the basis",
        published.wire_len
    );
    assert_eq!(apply(&mut consumer, &published), third);
}
