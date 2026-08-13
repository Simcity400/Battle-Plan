//! A commit site has to have written.
//!
//! The predecessor lane made the published cursor a type a staged number
//! cannot inhabit, and named the residual it could not close from there:
//! `commit_staged` took the CALLER'S WORD that the frame's bytes had reached
//! every authoritative sink. A caller that never wrote could still commit, and
//! the whole terminal-path lane is a list of arms that used to do exactly that
//! -- an `if let` with no else, a `_ => break`, a wildcard swallowing three
//! different endings.
//!
//! The receipt is what the word is replaced by. It is minted in one place, by
//! the function that hands the bytes to the sinks, only when every sink took
//! them, and it carries no number the caller supplied: both of its numbers are
//! read back out of the accepted buffer.
//!
//! Every test here fails if the published cursor can move without a write
//! behind it, if a receipt can be minted for a write that did not complete, or
//! if one frame's proof can be spent on another frame.

use std::io::Read;

use super::*;

/// An hour inside the compact calendar. The captured streams' first refused
/// hour, so the numbers here sit in the same range as the evidence.
const HOUR: i32 = 60_831_000;

fn temp_stream_path(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hoi4_collector_write_receipt_{tag}_{}_{}",
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
/// plain sink write and every byte lands in the file.
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

/// A transport whose pipe sink was requested and never opened: delivery is
/// terminal, so every write reports an INCOMPLETE outcome. This is the shape a
/// mixed-sink partial write has, and the one a receipt must never be minted
/// for.
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

/// `record_count` contiguous ordinary records starting at `first_sequence` --
/// the shape every compact frame has, numbered the way every builder numbers.
fn frame(first_sequence: u64, record_count: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    for index in 0..record_count {
        append_compact_record(
            &mut bytes,
            first_sequence + index,
            HOUR,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            b"receipt",
        );
    }
    bytes
}

/// The production source, read once. The mint and the commit doors are what
/// this module is about, and "there is exactly ONE mint" is a property of the
/// file, not of any value a test can hold.
const COLLECTOR_SOURCE: &str = include_str!("../attach_log.rs");

/// The same text with every `//` line comment removed. The receipt's own doc
/// comment names `seal_completed_write` several times; a count that read the
/// prose would be answering the documentation instead of the code.
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.split_once("//") {
            Some((code, _)) => code,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// What mints a receipt, and what cannot.
// ---------------------------------------------------------------------------

/// The whole point, at the smallest scale it can be stated: bytes that did not
/// reach every authoritative sink authorize nothing.
#[test]
fn a_write_that_did_not_reach_every_sink_mints_no_receipt() {
    let path = temp_stream_path("incomplete");
    let mut transport = refusing_transport(&path);

    let written = transport
        .write_compact_frame(&frame(1, 4))
        .expect("a refusing sink is not a transport error")
        .written()
        .expect("the bytes were offered to the sinks");

    assert!(
        !written.complete(),
        "the pipe sink was requested and never opened, so delivery is terminal"
    );
    assert!(
        written.into_receipt().is_none(),
        "an incomplete write carries nothing to commit with"
    );
    drop(transport);
}

/// The receipt's numbers are read off the wire, not taken from the caller. A
/// frame of 74 records numbered from 1 leaves the published cursor at 75 --
/// which is the captured stream's own prefix, sequences 1..=74 spent.
#[test]
fn a_receipt_numbers_the_frame_from_the_bytes_the_sinks_accepted() {
    let path = temp_stream_path("bounds");
    let mut transport = file_transport(&path);

    let receipt = transport
        .write_compact_frame(&frame(1, 74))
        .expect("legacy sinks are stable")
        .written()
        .expect("the bytes reached the sinks")
        .into_receipt()
        .expect("a complete write of numbered compact records mints a receipt");

    assert_eq!(
        receipt.staged_after_frame(),
        75,
        "74 records numbered from one leave the cursor at 75"
    );

    let mut cursor = PublishedSequence::start_on(None);
    cursor.commit_staged(receipt).expect("the frame was written");
    assert_eq!(cursor.get(), 75);
    drop(transport);
}

/// The two not-written verdicts are UNIT variants, and that is what makes the
/// arms that meet them uninhabitable rather than merely disciplined: there is
/// no value in them to take a receipt out of, so a commit cannot be written
/// there at all.
///
/// The runtime half of that is here; the compile half needs no test, because
/// it is not expressible: writing a commit inside the `RolledOver` arm has no
/// binding to take a receipt out of, so rustc answers `cannot find value` and
/// the shape never reaches a test run at all.
#[test]
fn a_not_written_verdict_carries_nothing_to_commit_with() {
    let path = temp_stream_path("not_written");
    let mut transport = file_transport(&path);

    transport.inject_campaign_identity_boundaries([
        CampaignIdentityBoundary::RetryUncertainBoundary,
    ]);
    assert!(
        transport
            .write_compact_frame(&frame(1, 4))
            .expect("an uncertain identity is a verdict, not an error")
            .written()
            .is_none(),
        "an uncertain boundary wrote nothing, so there is no sealed write at all"
    );
    drop(transport);
    assert!(
        read_stream(&path).is_empty(),
        "nothing reached the disk, which is what the verdict says"
    );
}

/// The legacy JSONL line paths write through the same door and move the cursor
/// with `advance_one_saturating`, not with a frame commit. A line is not a
/// numbered compact record, so it mints no receipt -- and must not, because a
/// receipt for it would carry sequence numbers parsed out of text.
#[test]
fn a_complete_write_of_bytes_that_are_not_compact_records_mints_no_receipt() {
    let sealed = seal_completed_write(
        SinkWriteOutcome {
            file_requested: true,
            file_complete: true,
            pipe_requested: false,
            pipe_complete: false,
        },
        b"{\"kind\":\"legacy_line\"}\n",
    )
    .expect("sealing a complete write of unnumbered bytes is not an error");
    assert!(sealed.complete());
    assert!(
        sealed.into_receipt().is_none(),
        "a JSONL line has no frame numbering to commit"
    );
}

// ---------------------------------------------------------------------------
// What a receipt may be spent on.
// ---------------------------------------------------------------------------

/// A receipt is proof about ONE frame. Offered to a cursor that frame was not
/// numbered from, it is refused -- and the refusal is not the rewind check
/// doing the work: this receipt is AHEAD of the cursor, exactly where the
/// rewind check cannot see it.
///
/// This is the cross-frame question stated at the only scale it can be: two
/// live receipts, from two real writes, spent in the wrong order.
#[test]
fn a_receipt_from_a_different_frame_is_refused_by_the_cursor_it_is_offered_to() {
    let path = temp_stream_path("cross_frame");
    let mut transport = file_transport(&path);

    let first = transport
        .write_compact_frame(&frame(1, 74))
        .expect("first frame")
        .written()
        .expect("the bytes reached the sinks")
        .into_receipt()
        .expect("a complete write mints a receipt");
    let second = transport
        .write_compact_frame(&frame(75, 74))
        .expect("second frame")
        .written()
        .expect("the bytes reached the sinks")
        .into_receipt()
        .expect("a complete write mints a receipt");

    let mut cursor = PublishedSequence::start_on(None);

    // The SECOND frame's proof, offered to a cursor still standing before the
    // first. Both frames really were written, so this is not a forgery -- it is
    // the confusion a `u64` argument could not distinguish from correctness.
    let error = cursor
        .commit_staged(second)
        .expect_err("a receipt for a frame numbered from 75 does not commit a cursor at 1");
    assert!(
        error.contains("belongs to a different frame"),
        "unexpected: {error}"
    );
    assert_eq!(cursor.get(), 1, "a refused commit spends nothing");

    cursor
        .commit_staged(first)
        .expect("the frame this cursor was numbered from commits");
    assert_eq!(cursor.get(), 75);
    drop(transport);
}

/// The receipt is the authority for where the cursor lands; the build's staged
/// number is a cross-check. They disagree only if the builder numbered records
/// it did not emit or emitted records it did not number -- the same class of
/// defect as the refusal stamp -- so the commit door refuses rather than
/// publishing over it.
#[test]
fn a_commit_whose_receipt_disagrees_with_the_build_is_refused() {
    let path = temp_stream_path("disagree");
    let mut transport = file_transport(&path);
    let mut cursor = PublishedSequence::start_on(None);

    let written = transport
        .write_compact_frame(&frame(1, 74))
        .expect("frame")
        .written()
        .expect("the bytes reached the sinks");

    // 222: the number the shipped producer's refused build reached. The frame
    // on the wire ends at 75.
    let error = commit_published_frame(&mut cursor, written, 222)
        .expect_err("a build that staged 222 did not write the frame that ends at 75");
    assert!(
        error.contains("leaves the published cursor at 75")
            && error.contains("its build staged 222"),
        "unexpected: {error}"
    );
    assert_eq!(cursor.get(), 1, "a refused commit spends nothing");
    drop(transport);
}

/// The one door, given a write that did not complete: it has no receipt, and
/// the door says so instead of moving the cursor.
#[test]
fn the_commit_door_refuses_a_write_that_minted_no_receipt() {
    let path = temp_stream_path("no_receipt");
    let mut transport = refusing_transport(&path);
    let mut cursor = PublishedSequence::start_on(None);

    let written = transport
        .write_compact_frame(&frame(1, 74))
        .expect("a refusing sink is not a transport error")
        .written()
        .expect("the bytes were offered to the sinks");

    let error = commit_published_frame(&mut cursor, written, 75)
        .expect_err("an incomplete write authorizes no commit");
    assert!(
        error.contains("without a write receipt"),
        "unexpected: {error}"
    );
    assert_eq!(cursor.get(), 1, "a refused commit spends nothing");
    drop(transport);
}

/// A refused hour spends EXACTLY one sequence -- the property the shipped
/// 75 -> 222 stamp broke. The single-record door states that arithmetic itself,
/// so a receipt for a whole frame cannot be spent as if it were one record.
#[test]
fn the_single_record_door_refuses_a_receipt_for_a_whole_frame() {
    let path = temp_stream_path("one_record");
    let mut transport = file_transport(&path);
    let mut cursor = PublishedSequence::start_on(None);

    let many = transport
        .write_compact_frame(&frame(1, 74))
        .expect("frame")
        .written()
        .expect("the bytes reached the sinks")
        .into_receipt()
        .expect("a complete write mints a receipt");

    let error = cursor
        .commit_one_published_record(many)
        .expect_err("74 records are not one record");
    assert!(
        error.contains("must spend exactly one sequence"),
        "unexpected: {error}"
    );
    assert_eq!(cursor.get(), 1, "a refused commit spends nothing");

    // A fresh stream for the one-record case: the legacy sink's own chain gate
    // requires the next batch to continue from 75, and this is about the door,
    // not about the gate.
    let single_path = temp_stream_path("one_record_single");
    let mut single = file_transport(&single_path);
    let one = single
        .write_compact_frame(&frame(1, 1))
        .expect("single record")
        .written()
        .expect("the bytes reached the sinks")
        .into_receipt()
        .expect("a complete write mints a receipt");
    cursor
        .commit_one_published_record(one)
        .expect("one written record spends one sequence");
    assert_eq!(cursor.get(), 2);
    drop(single);
    drop(transport);
}

// ---------------------------------------------------------------------------
// Properties of the FILE: there is one mint, and one way to move the cursor
// onto a frame.
// ---------------------------------------------------------------------------

/// The honest limit of the type, pinned so it stays the limit.
///
/// `seal_completed_write` is reachable from anywhere in this file, so the
/// receipt is unforgeable by CONSTRUCTION only as far as "no value you hold
/// yields one". A deliberate second call could mint for bytes no sink was ever
/// offered. That is not a wrong argument in a call -- it is a new call, and
/// this asserts there is exactly one, in the function that performs the write.
#[test]
fn exactly_one_production_call_site_mints_a_write_receipt() {
    let code = code_only(COLLECTOR_SOURCE);
    assert_eq!(
        code.matches("seal_completed_write(").count(),
        2,
        "the definition and ONE call: a second mint is a second place a receipt can exist \
         without a write behind it"
    );
    let write_fn = code
        .split_once("fn write(&mut self, bytes: &[u8]) -> Result<WrittenFrame, String> {")
        .expect("the transport write that hands bytes to the sinks")
        .1
        .split_once("\n    }")
        .expect("transport write terminator")
        .0;
    assert!(
        write_fn.contains("seal_completed_write(outcome, bytes)"),
        "the mint seals the outcome together with the exact buffer that was written"
    );
}

/// One door onto the published cursor for a frame, and the capture loop does
/// not open it by hand.
#[test]
fn every_frame_commit_goes_through_the_receipt_door() {
    let code = code_only(COLLECTOR_SOURCE);
    assert_eq!(
        code.matches("commit_staged(").count(),
        2,
        "the method and its ONE caller, `commit_published_frame`"
    );
    // Everything ahead of the receipt module: the capture worker with all
    // seven of its commit sites, and `finalize_scheduler_before_identity_
    // boundary` with its two. The cursor's own methods and their single caller
    // are declared after it.
    let before_the_receipt = code
        .split_once("mod frame_write_receipt {")
        .expect("the receipt module")
        .0;
    assert!(
        before_the_receipt.contains("fn write_attach_heartbeat("),
        "the capture worker is what this region has to cover"
    );
    assert!(
        !before_the_receipt.contains("commit_staged("),
        "no site inside the capture worker moves the published cursor with a number; every one \
         of them hands over the write"
    );
    assert!(
        !before_the_receipt.contains("advance_one_checked()"),
        "the single-record advance is reachable only through its receipt door"
    );
}

/// The scheduler-OFF write used to commit UNCONDITIONALLY, below the
/// `if !records.is_empty()` that guarded the write itself. It was harmless only
/// because an empty build stages nothing, and nothing checked that. A receipt
/// cannot be minted for a write that never happened, so the invariant is now
/// stated where it was assumed.
#[test]
fn an_empty_compact_frame_commits_nothing_and_says_why() {
    let code = code_only(COLLECTOR_SOURCE);
    let empty_branch = code
        .split_once("if records.is_empty() {")
        .expect("the scheduler-off empty-frame branch")
        .1
        .split_once("} else {")
        .expect("empty-frame branch terminator")
        .0;
    assert!(
        !empty_branch.contains("commit_published_frame")
            && !empty_branch.contains("commit_staged"),
        "an empty frame was not written and commits nothing"
    );
    assert!(
        empty_branch.contains("next_sequence != session_sequence.get()")
            && empty_branch.contains("return Err("),
        "an empty build whose staged cursor MOVED wrote records it never emitted; that fails the \
         run rather than passing silently"
    );
}

/// The receipt is a proof object, so duplicating it would duplicate the proof.
/// Asserted on the declaration for the same reason the cursor's missing `Copy`
/// is: the hole this closes is invisible to every value-level assertion,
/// because a copy behaves exactly like the original until it is spent twice.
#[test]
fn the_receipt_is_move_only() {
    let code = code_only(COLLECTOR_SOURCE);
    let declaration = code
        .split_once("pub(super) struct FrameWriteReceipt {")
        .expect("the receipt declaration")
        .0;
    let derives = declaration
        .rsplit_once("#[derive(")
        .expect("the receipt's derive list")
        .1
        .split_once(')')
        .expect("derive terminator")
        .0;
    assert!(
        !derives.contains("Copy") && !derives.contains("Clone") && !derives.contains("Default"),
        "a receipt that could be copied could authorize two commits from one write: {derives}"
    );
}
