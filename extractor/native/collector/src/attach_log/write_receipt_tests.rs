use std::io::Read;

use super::*;

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

const COLLECTOR_SOURCE: &str = include_str!("../attach_log.rs");

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
    cursor
        .commit_staged(receipt)
        .expect("the frame was written");
    assert_eq!(cursor.get(), 75);
    drop(transport);
}

#[test]
fn a_not_written_verdict_carries_nothing_to_commit_with() {
    let path = temp_stream_path("not_written");
    let mut transport = file_transport(&path);

    transport
        .inject_campaign_identity_boundaries([CampaignIdentityBoundary::RetryUncertainBoundary]);
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

#[test]
fn every_frame_commit_goes_through_the_receipt_door() {
    let code = code_only(COLLECTOR_SOURCE);
    assert_eq!(
        code.matches("commit_staged(").count(),
        2,
        "the method and its ONE caller, `commit_published_frame`"
    );
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
        !empty_branch.contains("commit_published_frame") && !empty_branch.contains("commit_staged"),
        "an empty frame was not written and commits nothing"
    );
    assert!(
        empty_branch.contains("next_sequence != session_sequence.get()")
            && empty_branch.contains("return Err("),
        "an empty build whose staged cursor MOVED wrote records it never emitted; that fails the \
         run rather than passing silently"
    );
}

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
