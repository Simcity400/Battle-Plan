//! A refused hour spends exactly one stream sequence: the one its `H4FR`
//! record occupies.
//!
//! The three captured streams on disk all break the same way. `deflist-02`
//! walks as 73 `DOMAIN_GAP` records on sequences 1..=73, a `FRAME_END` on 74,
//! and then 270 `FRAME_REFUSED` records stamped 202..=222 — repeating and
//! running backwards. The certifier stops at the first of them with
//! "compact record sequence 222 is not the next required sequence 75", and it
//! is right to: the chain has holes.
//!
//! The cause is one number in the wrong place. The refused frame's build had
//! already numbered its staged DATA/PERF records — 147 of them, cursor 75 ->
//! 222 — and the refusal record was stamped with that STAGED cursor. Those 147
//! records are then discarded, so they never reach a sink; the published cursor
//! correctly does not move, and the next refused hour restages from 75 again
//! and stamps whatever its own staged count happens to reach. That is both
//! symptoms: the 74 -> 222 jump AND the repeats and reversals that follow.
//!
//! The withholding is legitimate and is unchanged here. A refused frame's
//! per-domain readings are void under the all-or-nothing guarantee, and the
//! `H4FR` record already accounts for the hour and names the offenders with
//! their bits and typed reasons. What the stream must not do is spend numbers
//! on bytes it never emits.
//!
//! Every test here fails if a published record is numbered from anything other
//! than the cursor of records that actually reached a sink.

use std::collections::BTreeMap;
use std::io::Read;

use compact_contract::frame_refusal::FrameRefusalOffender;
use compact_contract::stream_validator::{StatefulStreamValidator, StreamValidationError};

use super::*;
use crate::read_only_scheduler::{DomainGapRange, GapKind};

/// The hour `deflist-02` first refused.
const MEASURED_FIRST_REFUSED_HOUR: i32 = 60_831_000;
/// The published cursor at that moment: one 73-domain H4DG frame and its
/// FRAME_END had reached the sink, so sequences 1..=74 were spent.
const MEASURED_PUBLISHED_CURSOR: u64 = 75;
/// The cursor that refused frame's BUILD returned — 147 staged records that
/// were then discarded. The shipped producer stamped the H4FR with this.
const MEASURED_STAGED_CURSOR: u64 = 222;

fn temp_stream_path(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hoi4_collector_frame_refusal_sequence_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp stream dir");
    dir.join("collector_stream.bin")
}

/// A legacy transport whose only authoritative sink is `path`. Legacy sinks
/// report `CampaignIdentityBoundary::Stable`, so `write_compact_frame` is the
/// plain sink write and every byte the helper publishes lands in the file.
fn file_transport(path: &std::path::Path) -> CaptureTransport {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .expect("open temp stream");
    CaptureTransport::legacy(
        Some(file),
        None,
        SinkDeliveryState::from_open_results(true, true, false, false),
    )
}

/// A transport whose pipe sink was requested and never opened, so delivery is
/// terminal and every write reports an incomplete outcome.
fn refusing_transport(path: &std::path::Path) -> CaptureTransport {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .expect("open temp stream");
    CaptureTransport::legacy(
        Some(file),
        None,
        SinkDeliveryState::from_open_results(true, true, true, false),
    )
}

fn read_stream(path: &std::path::Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .expect("reopen temp stream")
        .read_to_end(&mut bytes)
        .expect("read temp stream");
    bytes
}

/// Walk a whole stream the way the decoder does and return every accepted
/// header, or the exact validation error that stopped it.
fn certify(
    bytes: &[u8],
) -> Result<Vec<compact_contract::record_header::CompactRecordHeader>, StreamValidationError> {
    let mut validator = StatefulStreamValidator::legacy();
    let mut headers = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let header = compact_contract::parse_compact_record_header(&bytes[offset..])
            .expect("stream framing must be walkable");
        let payload_start = offset + compact_contract::record_header::COMPACT_RECORD_HEADER_LEN;
        let payload = &bytes[payload_start..payload_start + header.payload_len];
        validator.validate_record(&header, payload)?;
        headers.push(header);
        offset = payload_start + header.payload_len;
    }
    Ok(headers)
}

/// The exact roster shape the captured streams carry: 73 scheduled domains.
fn roster() -> Vec<u16> {
    (10u16..83u16).collect()
}

/// One H4DG frame exhausting the scheduled roster over `hour..=hour`, built by
/// the production builder so the prefix is a real frame and not a hand-rolled
/// approximation.
fn gap_frame(
    hour: i32,
    cursor: u64,
    ordinals: &mut BTreeMap<(u16, u16), u64>,
) -> (Vec<u8>, u64) {
    let roster = roster();
    let gaps: Vec<DomainGapRange> = roster
        .iter()
        .map(|domain_id| DomainGapRange {
            domain_id: *domain_id,
            epoch: 1,
            first_hour_raw: hour,
            last_hour_raw: hour,
            kind: GapKind::ClockUnreadable,
            force_next_baseline: false,
            tick_before: 0,
            tick_after: 0,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
        })
        .collect();
    let mut frames = build_scheduler_gap_frames(&gaps, &roster, cursor, 0, ordinals)
        .expect("scheduler gap frame")
        .into_iter();
    let frame = frames.next().expect("one H4DG frame");
    assert!(frames.next().is_none(), "one gap range is one frame");
    frame
}

/// A published cursor standing exactly where a committed prefix left it.
///
/// `PublishedSequence` has no constructor from an arbitrary number on purpose --
/// that absence is what makes a staged cursor unable to reach a publishing call
/// site. The one legitimate crossing is `commit_staged`, which now consumes the
/// receipt a completed write mints, and that is precisely what a committed
/// prefix means here: a frame numbered from sequence one that reached every
/// sink and left the cursor where the caller asks for it.
fn published_at(cursor_after_committed_prefix: u64) -> PublishedSequence {
    // A legacy stream: exactly one logical stream, and it never rolls.
    let mut cursor = PublishedSequence::start_on(None);
    cursor
        .commit_staged(FrameWriteReceipt::for_cursor_arithmetic_test(
            1,
            cursor_after_committed_prefix,
        ))
        .expect("a committed prefix never leaves the cursor behind its own start");
    cursor
}

fn offenders(count: usize) -> Vec<FrameRefusalOffender> {
    (0..count)
        .map(|index| FrameRefusalOffender {
            domain_id: 10 + index as u16,
            status_flags: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            non_tolerated_flags: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            reason: format!("d{}_source_unavailable=1", 10 + index),
        })
        .collect()
}

/// Reproduces the captured prefix exactly: 73 gaps on 1..=73, FRAME_END on 74.
/// If this drifts, the constants above stop describing the evidence and the
/// rest of the module is measuring something else.
#[test]
fn the_gap_prefix_reproduces_the_captured_streams_first_seventy_four_sequences() {
    let mut ordinals = BTreeMap::new();
    let (prefix, cursor) = gap_frame(60_830_999, 1, &mut ordinals);
    let headers = certify(&prefix).expect("the captured prefix certifies");
    assert_eq!(headers.len(), 74, "73 domain gaps and one FRAME_END");
    assert_eq!(
        headers.first().map(|header| header.sequence),
        Some(1),
        "the first ordinary record is sequence one"
    );
    assert_eq!(
        headers.last().map(|header| (header.sequence, header.record_type)),
        Some((74, compact_contract::COMPACT_RECORD_TYPE_FRAME_END)),
        "FRAME_END takes 74, exactly as the captured streams do"
    );
    assert_eq!(
        cursor, MEASURED_PUBLISHED_CURSOR,
        "the published cursor after the prefix is the one the captures stood at"
    );
}

/// The whole shape the captures could never reach: prefix, then refused hours
/// on consecutive sequences, certified end to end by the same stateful
/// validator the certifier uses.
#[test]
fn refused_hours_take_consecutive_published_sequences_and_the_stream_certifies() {
    let path = temp_stream_path("consecutive");
    let mut transport = file_transport(&path);
    let mut ordinals = BTreeMap::new();

    let (prefix, cursor_after_prefix) = gap_frame(60_830_999, 1, &mut ordinals);
    let mut cursor = published_at(cursor_after_prefix);
    assert!(
        transport
            .write_compact_frame(&prefix)
            .expect("prefix write")
            .written()
            .expect("legacy sinks are always stable")
            .complete(),
        "the prefix frame must reach the sink"
    );

    // Offender counts differ per hour on purpose: in the captured streams the
    // stamp moved with the refused frame's own size. It must not here.
    for (index, offender_count) in [1usize, 17, 28].into_iter().enumerate() {
        let before = cursor.get();
        let outcome = publish_frame_refusal_record(
            &mut transport,
            &mut cursor,
            MEASURED_FIRST_REFUSED_HOUR + index as i32,
            offenders(offender_count),
        )
        .expect("refusal publishes");
        assert_eq!(outcome, FrameRefusalPublication::Published);
        assert_eq!(
            cursor.get(),
            before + 1,
            "a refused hour spends exactly one sequence regardless of how many \
             records its frame staged"
        );
    }
    assert_eq!(cursor.get(), MEASURED_PUBLISHED_CURSOR + 3);
    drop(transport);

    let headers = certify(&read_stream(&path)).expect("the published stream certifies");
    assert_eq!(headers.len(), 77, "74 prefix records and three refusals");
    let refusals: Vec<_> = headers
        .iter()
        .filter(|header| {
            header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED
        })
        .map(|header| (header.sequence, header.game_hour_raw, header.domain_id))
        .collect();
    assert_eq!(
        refusals,
        vec![
            (75, MEASURED_FIRST_REFUSED_HOUR, compact_contract::COMPACT_DOMAIN_FRAME),
            (76, MEASURED_FIRST_REFUSED_HOUR + 1, compact_contract::COMPACT_DOMAIN_FRAME),
            (77, MEASURED_FIRST_REFUSED_HOUR + 2, compact_contract::COMPACT_DOMAIN_FRAME),
        ],
        "each refused hour is numbered from the published cursor, in order"
    );
}

/// The cursor a refusal leaves behind is the cursor the next real frame builds
/// from. This is the handoff the captured streams never performed: an H4DG
/// frame after a refusal has to continue the same unbroken chain.
#[test]
fn a_refused_hour_between_two_published_frames_keeps_the_chain_unbroken() {
    let path = temp_stream_path("interleaved");
    let mut transport = file_transport(&path);
    let mut ordinals = BTreeMap::new();

    let (first, cursor_after_first) = gap_frame(60_830_999, 1, &mut ordinals);
    let mut cursor = published_at(cursor_after_first);
    transport.write_compact_frame(&first).expect("first frame");

    publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        offenders(3),
    )
    .expect("refusal publishes");

    let (second, cursor_after_second) = gap_frame(60_831_001, cursor.get(), &mut ordinals);
    let second_write = transport
        .write_compact_frame(&second)
        .expect("second frame")
        .written()
        .expect("legacy sinks are stable");
    commit_published_frame(&mut cursor, second_write, cursor_after_second)
        .expect("the second frame reached the sink");

    publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        60_831_002,
        offenders(9),
    )
    .expect("second refusal publishes");
    drop(transport);

    let headers = certify(&read_stream(&path)).expect("the interleaved stream certifies");
    assert_eq!(headers.len(), 74 + 1 + 74 + 1);
    assert_eq!(
        cursor.get(),
        151,
        "74 + 1 + 74 + 1 records spend sequences 1..=150"
    );
    let refusal_sequences: Vec<u64> = headers
        .iter()
        .filter(|header| {
            header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED
        })
        .map(|header| header.sequence)
        .collect();
    assert_eq!(
        refusal_sequences,
        vec![75, 150],
        "a refusal takes the next published sequence on both sides of a real frame"
    );
}

/// A record that did not reach every authoritative sink did not happen. The
/// cursor must not move, or the next record leaves a hole exactly like the one
/// this lane is closing.
#[test]
fn a_refusal_no_sink_accepted_does_not_advance_the_published_cursor() {
    let path = temp_stream_path("incomplete");
    let mut transport = refusing_transport(&path);
    let mut cursor = published_at(MEASURED_PUBLISHED_CURSOR);

    let outcome = publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        offenders(4),
    )
    .expect("an incomplete write is an outcome, not an error");

    assert_eq!(outcome, FrameRefusalPublication::Incomplete);
    assert_eq!(
        cursor.get(),
        MEASURED_PUBLISHED_CURSOR,
        "an unaccepted record spends no sequence"
    );
    drop(transport);
    assert!(
        read_stream(&path).is_empty(),
        "a terminal delivery admits no bytes"
    );
}

/// A refusal that cannot be stated is refused outright rather than published as
/// an empty assertion, and it spends no sequence on the way out.
#[test]
fn a_refusal_naming_no_offender_is_rejected_and_spends_no_sequence() {
    let path = temp_stream_path("no_offender");
    let mut transport = file_transport(&path);
    let mut cursor = published_at(MEASURED_PUBLISHED_CURSOR);

    let error = publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        Vec::new(),
    )
    .expect_err("a refusal naming nobody is incoherent");

    assert!(
        error.contains("names no offending domain"),
        "unexpected error: {error}"
    );
    assert_eq!(cursor.get(), MEASURED_PUBLISHED_CURSOR);
    drop(transport);
    assert!(read_stream(&path).is_empty());
}

/// The producer refuses to write the hole it was about to write.
///
/// The `IdentityV1` barrier has always enforced this in
/// `write_ordinary_records`; the legacy sink enforced nothing, and all three
/// captured streams are legacy. So the whole shipped defect went to disk
/// unchallenged and was only found by the certifier afterwards, when the
/// capture was already spent. This drives the exact measured numbers: 1..=74
/// committed, then a record stamped 222.
#[test]
fn a_legacy_sink_refuses_a_compact_batch_that_does_not_continue_the_chain() {
    let path = temp_stream_path("chain_gate");
    let mut transport = file_transport(&path);
    let mut ordinals = BTreeMap::new();

    let (prefix, _) = gap_frame(60_830_999, 1, &mut ordinals);
    transport
        .write_compact_frame(&prefix)
        .expect("prefix write")
        .written()
        .expect("legacy sinks are stable");

    // The shipped producer's number, offered to the sink that used to take it.
    // `commit_staged` is the assertion the shipped producer made in effect: that
    // the refused frame's staged cursor had been committed. It had not.
    let mut staged_cursor = published_at(MEASURED_STAGED_CURSOR);
    let error = publish_frame_refusal_record(
        &mut transport,
        &mut staged_cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        offenders(28),
    )
    .expect_err("the sink must refuse a record that does not continue the chain");

    assert!(
        error.contains("222 is not the next required sequence 75"),
        "unexpected error: {error}"
    );
    assert_eq!(
        staged_cursor.get(),
        MEASURED_STAGED_CURSOR,
        "a refused write spends no sequence"
    );
    drop(transport);
    let headers = certify(&read_stream(&path)).expect("only the prefix reached the file");
    assert_eq!(
        headers.len(),
        74,
        "the holed record must not be on the disk at all"
    );
}

/// The gate is on the whole batch, not just its first record: a frame whose
/// records repeat or skip internally is refused before any byte is written.
#[test]
fn a_legacy_sink_refuses_a_batch_whose_own_records_are_not_contiguous() {
    let path = temp_stream_path("batch_gate");
    let mut transport = file_transport(&path);

    let payload = compact_contract::frame_refusal::FrameRefusalRecordV1 {
        refused_hour: MEASURED_FIRST_REFUSED_HOUR,
        offenders: offenders(2),
    }
    .encode()
    .expect("refusal encodes");
    let mut batch = Vec::new();
    for sequence in [1u64, 1, 2] {
        append_compact_record(
            &mut batch,
            sequence,
            MEASURED_FIRST_REFUSED_HOUR,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &payload,
        );
    }

    let error = transport
        .write_compact_frame(&batch)
        .expect_err("a batch that repeats a sequence is refused");
    assert!(
        error.contains("is not the next required sequence 2"),
        "unexpected error: {error}"
    );
    drop(transport);
    assert!(
        read_stream(&path).is_empty(),
        "a refused batch writes nothing"
    );
}

/// The gate must not touch the legacy JSONL line path, which shares this sink
/// and carries no sequence at all.
#[test]
fn the_chain_gate_passes_non_compact_legacy_lines_through() {
    let path = temp_stream_path("jsonl");
    let mut transport = file_transport(&path);
    for line in ["{\"a\":1}\n", "{\"a\":2}\n"] {
        assert!(
            transport
                .write(line.as_bytes())
                .expect("legacy line write")
                .complete(),
            "a non-compact line is not a chain record"
        );
    }
    drop(transport);
    assert_eq!(read_stream(&path), b"{\"a\":1}\n{\"a\":2}\n");
}

/// The measured defect, reconstructed: the same prefix, then a refusal stamped
/// from the STAGED cursor the way the shipped producer stamped it. The chain
/// check must refuse it with the exact error the certifier reported at byte
/// 7656 — and must refuse the repeat that follows for the same reason.
///
/// This is the guard on the decoder side of the hard constraint: if the
/// sequence check is ever loosened to tolerate a holed chain, this goes red.
#[test]
fn the_captured_streams_staged_numbering_is_what_the_chain_check_refuses() {
    let mut ordinals = BTreeMap::new();
    let (mut stream, cursor) = gap_frame(60_830_999, 1, &mut ordinals);
    assert_eq!(cursor, MEASURED_PUBLISHED_CURSOR);

    let payload = compact_contract::frame_refusal::FrameRefusalRecordV1 {
        refused_hour: MEASURED_FIRST_REFUSED_HOUR,
        offenders: offenders(28),
    }
    .encode()
    .expect("refusal encodes");
    // Exactly what the shipped producer did: the staged cursor, twice, because
    // the published cursor never moved.
    for hour_offset in 0..2i32 {
        append_compact_record(
            &mut stream,
            MEASURED_STAGED_CURSOR,
            MEASURED_FIRST_REFUSED_HOUR + hour_offset,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &payload,
        );
    }

    assert_eq!(
        certify(&stream),
        Err(StreamValidationError::UnexpectedSequence {
            expected: MEASURED_PUBLISHED_CURSOR,
            actual: MEASURED_STAGED_CURSOR,
        }),
        "the chain check must refuse a stream whose producer spent sequences on \
         records it withheld"
    );
}

// ---------------------------------------------------------------------------
// The published cursor as a TYPE, and the two rewinds that reach it.
//
// The stamp defect was one argument in one call, on a line only a live capture
// loop can reach. `PublishedSequence` is what stops that argument existing. The
// tests below pin what its methods promise; the compile error itself cannot be
// a test here, because a test that fails to compile is not a test.
// ---------------------------------------------------------------------------

/// A committed frame's staged cursor is at or ahead of the published one,
/// always -- the build seeded from the published cursor and counted up. Behind
/// it means the frame was numbered from something else, which is the stamp
/// defect in a different place, so the crossing refuses rather than rewinding.
#[test]
fn committing_a_staged_cursor_behind_the_published_one_is_refused() {
    let mut cursor = published_at(MEASURED_PUBLISHED_CURSOR);

    // A STALE receipt: proof for a frame numbered from sequence one, offered to
    // a cursor this stream has already carried to 74.
    let error = cursor
        .commit_staged(FrameWriteReceipt::for_cursor_arithmetic_test(
            1,
            MEASURED_PUBLISHED_CURSOR - 1,
        ))
        .expect_err("a staged cursor behind the published one would rewind the chain");
    assert!(error.contains("the chain would rewind"), "unexpected: {error}");
    assert_eq!(
        cursor.get(),
        MEASURED_PUBLISHED_CURSOR,
        "a refused commit spends nothing"
    );

    // Ahead and equal are both ordinary. Equal is an empty frame: it staged
    // nothing, so it committed nothing -- and a real write can no longer even
    // produce that receipt, because a write of zero records mints none.
    cursor
        .commit_staged(FrameWriteReceipt::for_cursor_arithmetic_test(
            MEASURED_PUBLISHED_CURSOR,
            MEASURED_PUBLISHED_CURSOR,
        ))
        .expect("an empty committed frame leaves the cursor where it was");
    assert_eq!(cursor.get(), MEASURED_PUBLISHED_CURSOR);
    cursor
        .commit_staged(FrameWriteReceipt::for_cursor_arithmetic_test(
            MEASURED_PUBLISHED_CURSOR,
            MEASURED_PUBLISHED_CURSOR + 147,
        ))
        .expect("a committed frame of 147 records advances by 147");
    assert_eq!(cursor.get(), MEASURED_PUBLISHED_CURSOR + 147);
}

/// The `attach_log.rs:3013` rewind, at the level that decides it.
///
/// A same-identity forward reload replaces the CGameState under every cached
/// root, so it runs the campaign-scoped reset -- but the IdentityV1 barrier
/// still holds the logical stream it opened and no CID1 was acknowledged. The
/// reset used to restart numbering anyway, which would put a record stamped 1
/// after one stamped 74.
#[test]
fn a_reset_on_the_same_logical_stream_does_not_rewind_the_published_cursor() {
    let mut cursor = PublishedSequence::start_on(Some(7));
    cursor
        .commit_staged(FrameWriteReceipt::for_cursor_arithmetic_test(
            1,
            MEASURED_PUBLISHED_CURSOR,
        ))
        .expect("the prefix committed");

    let restarted = cursor.resync_to_logical_stream(Some(7));

    assert!(
        !restarted,
        "the barrier still holds logical stream 7; nothing rolled"
    );
    assert_eq!(
        cursor.get(),
        MEASURED_PUBLISHED_CURSOR,
        "the chain continues from where it stood"
    );
}

/// The other side of the same decision: a rollover REPLACES the barrier with
/// one staged on a newly acknowledged id, and that stream's first ordinary
/// record is sequence 1.
#[test]
fn a_reset_onto_a_new_logical_stream_restarts_numbering_at_one() {
    let mut cursor = PublishedSequence::start_on(Some(7));
    cursor
        .commit_staged(FrameWriteReceipt::for_cursor_arithmetic_test(
            1,
            MEASURED_PUBLISHED_CURSOR,
        ))
        .expect("the prefix committed");

    let restarted = cursor.resync_to_logical_stream(Some(8));

    assert!(restarted, "logical stream 8 is not logical stream 7");
    assert_eq!(cursor.get(), 1, "a new logical stream starts at sequence 1");
}

/// A legacy stream has exactly one logical stream and never rolls. All three
/// captured streams on disk are legacy, so this is the shape the evidence has.
#[test]
fn a_legacy_stream_is_never_restarted_by_a_campaign_reset() {
    let mut cursor = PublishedSequence::start_on(None);
    cursor
        .commit_staged(FrameWriteReceipt::for_cursor_arithmetic_test(
            1,
            MEASURED_PUBLISHED_CURSOR,
        ))
        .expect("the prefix committed");

    for _ in 0..3 {
        assert!(
            !cursor.resync_to_logical_stream(None),
            "a legacy transport reports no logical stream, ever"
        );
    }
    assert_eq!(cursor.get(), MEASURED_PUBLISHED_CURSOR);
}

/// A cursor that cannot advance must stop the stream, not wrap onto a number
/// another record already carries.
#[test]
fn a_published_cursor_at_the_maximum_refuses_to_advance() {
    let mut cursor = published_at(u64::MAX);
    assert!(cursor.checked_successor().is_err());
    assert!(cursor.advance_one_checked().is_err());
    assert_eq!(cursor.get(), u64::MAX, "a refused advance spends nothing");
}

/// The successor is reserved BEFORE any byte is written, so a cursor that
/// cannot be advanced stops the stream instead of publishing a record whose
/// successor has no number. Assert on the FILE: the point is what did not reach
/// the disk, not what the function returned.
#[test]
fn a_refusal_that_could_not_be_numbered_next_writes_no_bytes_at_all() {
    let path = temp_stream_path("no_successor");
    let mut transport = file_transport(&path);
    let mut cursor = published_at(u64::MAX);

    let error = publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        offenders(2),
    )
    .expect_err("a record whose successor has no number must not be published");
    // Assert the PRODUCER's own refusal, not merely "some overflow error".
    // The legacy sink's chain gate has an independent overflow check whose
    // message also ends in "without overflow", so a substring that broad stays
    // green when the reservation here is deleted -- the sink catches it one
    // step later instead, after `append_compact_record` has already built the
    // record. Pinning the producer's wording is what makes the reserve-BEFORE-
    // write guard provable rather than merely redundant with the sink.
    assert!(
        error.starts_with("frame refusal record cannot advance the compact sequence"),
        "the refusal must fail on its own reservation, not on the sink's later \
         chain check: {error}"
    );
    assert_eq!(cursor.get(), u64::MAX);
    drop(transport);
    assert!(
        read_stream(&path).is_empty(),
        "the record must not reach the disk before the cursor is found unadvanceable"
    );
}

/// The legacy line paths stamp straight from the published cursor and advance
/// by one per accepted line. This pins the arithmetic those sites rely on; the
/// sites themselves are inside the capture loop and are not reachable here.
#[test]
fn an_accepted_record_advances_the_published_cursor_by_exactly_one() {
    let mut cursor = published_at(MEASURED_PUBLISHED_CURSOR);
    for offset in 1..=5u64 {
        cursor.advance_one_saturating();
        assert_eq!(cursor.get(), MEASURED_PUBLISHED_CURSOR + offset);
    }
}

/// A legacy transport has no logical stream id, which is what makes every
/// campaign reset on it a no-op for numbering. All three captured streams on
/// disk are legacy.
#[test]
fn a_legacy_transport_reports_no_logical_stream_id() {
    let path = temp_stream_path("legacy_stream_id");
    let transport = file_transport(&path);
    assert_eq!(transport.current_logical_stream_id(), None);
}

/// An IdentityV1 transport whose barrier holds `logical_stream_id`, with a file
/// as its only authoritative sink.
///
/// The pipe type parameter is named but never instantiated:
/// `FreshIdentitySinkPermit::inspect` accepts a file-only permit, which is what
/// lets an IdentityV1 transport exist here without a live collector pipe. The
/// transport is built by struct literal rather than through
/// `CaptureTransport::identity_v1` so that no `OrdinaryAutoAttachControl` --
/// and so no control thread -- is needed to read one accessor.
fn identity_v1_transport(logical_stream_id: u64, path: &std::path::Path) -> CaptureTransport {
    use compact_contract::campaign_identity::{
        encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
    };
    let game_hour_raw = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24;
    let observation = Cid1Observation {
        game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
        game_unique_seed: -1234,
        game_hour_raw,
    };
    let fields = Cid1Fields::from_stable_observations(
        observation.clone(),
        observation,
        "test-profile".to_string(),
        Cid1Witnesses::default(),
    )
    .expect("two stable observations make a valid CID1");
    let payload = encode_cid1(&fields).expect("encode CID1");
    let mut cid1 = Vec::new();
    append_compact_record(
        &mut cid1,
        0,
        game_hour_raw,
        COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
        0,
        &payload,
    );
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(path)
        .expect("open temp stream");
    let permit: FreshIdentitySinkPermit<OwnedCollectorPipe> =
        FreshIdentitySinkPermit::inspect(Some(file), None).expect("file-only IdentityV1 permit");
    let barrier = IdentityV1ProducerBarrier::stage(logical_stream_id, &cid1, permit)
        .expect("stage the IdentityV1 barrier on its CID1");
    CaptureTransport {
        sinks: Some(AuthoritativeSinkSet::IdentityV1 {
            barrier,
            progress: Arc::new(AtomicU64::new(0)),
            last_identity_hour_raw: game_hour_raw,
            read_context: CampaignIdentityReadContext {
                base: 0,
                daily_match: true,
                long_string_match: true,
                date_ptr: 0,
            },
        }),
        ordinary_control: None,
        injected_boundaries: std::collections::VecDeque::new(),
    }
}

/// An IdentityV1 transport reports the logical stream its barrier holds, and
/// that report is what restarts numbering.
///
/// This is the other half of the legacy case above, and it is the half with
/// teeth: `PublishedSequence::resync_to_logical_stream` decides purely on this
/// accessor, so a transport that reported `None` here would make a GENUINE
/// campaign rollover a numbering no-op. The new logical stream would then carry
/// on the closed stream's numbers instead of restarting at 1 -- a record
/// stamped 75 as the first record of a stream whose first record must be 1.
#[test]
fn an_identity_v1_transport_reports_the_logical_stream_that_restarts_numbering() {
    let path = temp_stream_path("identity_stream_id");
    let transport = identity_v1_transport(4242, &path);
    assert_eq!(
        transport.current_logical_stream_id(),
        Some(4242),
        "an IdentityV1 transport must report its barrier's logical stream"
    );

    // A cursor that had been numbering some other stream restarts on this one,
    // and it restarts because of what the accessor returned.
    let mut cursor = published_at(MEASURED_PUBLISHED_CURSOR);
    assert!(
        cursor.resync_to_logical_stream(transport.current_logical_stream_id()),
        "moving onto a different logical stream restarts numbering"
    );
    assert_eq!(cursor.get(), 1, "a new logical stream begins at sequence 1");
}

// ---------------------------------------------------------------------------
// Retry is not rollover.
//
// `write_compact_frame` used to return `None` for both, and every caller did
// the rollover thing for both: restart numbering, drop delta baselines, tear
// down and reinstall hook owners -- because one identity read could not be
// confirmed twice running.
//
// A `RetryUncertainBoundary` verdict comes only from an IdentityV1 transport
// whose two stability reads of a LIVE CGameState disagree. Reproducing that for
// real needs a live patched hoi4.exe, which no test may launch, so the verdict
// is injected. Everything downstream of it -- `write_compact_frame`,
// `publish_frame_refusal_record`, the cursor, the sink -- is production code.
// ---------------------------------------------------------------------------

/// The two not-written verdicts are distinguishable at the transport boundary,
/// and the uncertain one writes nothing at all.
#[test]
fn an_uncertain_boundary_writes_nothing_and_is_not_a_rollover() {
    let path = temp_stream_path("uncertain_write");
    let mut transport = file_transport(&path);
    let mut ordinals = BTreeMap::new();
    let (frame, _) = gap_frame(60_830_999, 1, &mut ordinals);

    transport
        .inject_campaign_identity_boundaries([CampaignIdentityBoundary::RetryUncertainBoundary]);

    assert!(
        matches!(
            transport.write_compact_frame(&frame),
            Ok(CompactFrameWrite::RetryUncertainBoundary)
        ),
        "an uncertain identity is its own verdict, not a rollover"
    );
    drop(transport);
    assert!(
        read_stream(&path).is_empty(),
        "an uncertain boundary wrote nothing"
    );
}

/// A refused hour is still owed when the identity will not settle. Nothing
/// changed, so the publication re-attempts, and when the identity settles the
/// hour takes the sequence it would have taken all along -- exactly one.
#[test]
fn a_refusal_re_attempts_across_an_uncertain_boundary_and_still_spends_one_sequence() {
    let path = temp_stream_path("uncertain_refusal");
    let mut transport = file_transport(&path);
    let mut ordinals = BTreeMap::new();

    let (prefix, cursor_after_prefix) = gap_frame(60_830_999, 1, &mut ordinals);
    transport
        .write_compact_frame(&prefix)
        .expect("prefix write")
        .written()
        .expect("legacy sinks are stable");
    let mut cursor = published_at(cursor_after_prefix);

    transport.inject_campaign_identity_boundaries([
        CampaignIdentityBoundary::RetryUncertainBoundary,
        CampaignIdentityBoundary::RetryUncertainBoundary,
    ]);

    let outcome = publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        offenders(28),
    )
    .expect("an uncertain identity is not a failure");

    assert_eq!(outcome, FrameRefusalPublication::Published);
    assert_eq!(
        cursor.get(),
        MEASURED_PUBLISHED_CURSOR + 1,
        "two uncertain reads cost the stream no sequences"
    );
    drop(transport);

    let headers = certify(&read_stream(&path)).expect("the stream certifies");
    assert_eq!(
        headers.len(),
        75,
        "74 prefix records and exactly ONE refusal -- not one per attempt"
    );
    assert_eq!(
        headers.last().map(|header| header.sequence),
        Some(MEASURED_PUBLISHED_CURSOR)
    );
}

/// An identity that never settles is terminal, not a silently dropped hour. The
/// capture loop can retry the same verdict forever because its own deadline
/// bounds it; a publication holding a record the stream is owed cannot.
#[test]
fn a_refusal_gives_up_on_an_identity_that_never_settles_and_spends_no_sequence() {
    let path = temp_stream_path("never_settles");
    let mut transport = file_transport(&path);
    let mut cursor = published_at(MEASURED_PUBLISHED_CURSOR);

    transport.inject_campaign_identity_boundaries(
        (0..FRAME_REFUSAL_UNCERTAIN_BOUNDARY_ATTEMPTS)
            .map(|_| CampaignIdentityBoundary::RetryUncertainBoundary),
    );

    let error = publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        offenders(2),
    )
    .expect_err("an hour that cannot be stated must stop the run");

    assert!(
        error.contains("stayed uncertain"),
        "the cause must name the uncertain identity, got: {error}"
    );
    assert_eq!(
        cursor.get(),
        MEASURED_PUBLISHED_CURSOR,
        "an unpublished refusal spends no sequence"
    );
    drop(transport);
    assert!(read_stream(&path).is_empty());
}

/// A sink refusal is decided on the first attempt and never retried into: it is
/// terminal, and it must stay distinguishable from the uncertain verdict above,
/// which is not terminal at all.
#[test]
fn a_sink_refusal_is_terminal_on_the_first_attempt_and_is_not_retried() {
    let path = temp_stream_path("not_retried");
    let mut transport = refusing_transport(&path);
    let mut cursor = published_at(MEASURED_PUBLISHED_CURSOR);

    let outcome = publish_frame_refusal_record(
        &mut transport,
        &mut cursor,
        MEASURED_FIRST_REFUSED_HOUR,
        offenders(2),
    )
    .expect("an incomplete write is an outcome, not an error");

    assert_eq!(outcome, FrameRefusalPublication::Incomplete);
    assert_eq!(cursor.get(), MEASURED_PUBLISHED_CURSOR);
}

/// The published cursor is move-only, asserted on the DECLARATION.
///
/// This lane's battery entry M13 used to prove it by mutation: take a copy of
/// the live cursor, walk the copy onto the staged number, and publish from the
/// copy. That mutation no longer compiles -- but not because of `Copy`. Since
/// `commit_staged` takes a write receipt, the laundering line is a type error
/// on its ARGUMENT, and it would stay a type error with `Copy` restored. The
/// proof went stale without going red, which is the one failure mode a
/// mutation battery must not have.
///
/// So the property is asserted where nothing else can stand in for it. A copy
/// of a cursor behaves exactly like the original until it is spent twice, so no
/// value-level test can see the difference; the derive list can.
#[test]
fn the_published_cursor_is_move_only() {
    const COLLECTOR_SOURCE: &str = include_str!("../attach_log.rs");
    let declaration = COLLECTOR_SOURCE
        .split_once("    pub(super) struct PublishedSequence {")
        .expect("the published cursor declaration")
        .0;
    let derives = declaration
        .rsplit_once("#[derive(")
        .expect("the cursor's derive list")
        .1
        .split_once(')')
        .expect("derive terminator")
        .0;
    assert!(
        !derives.contains("Copy") && !derives.contains("Clone"),
        "a cursor that could be duplicated could be walked onto a staged number on the copy and \
         published from it, while every assertion watched the original stand still: {derives}"
    );
}
