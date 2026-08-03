//! The terminal (shutdown) write path publishes what it is holding, or names
//! what it could not.
//!
//! A capture ends at an arbitrary moment -- the host asks it to stop, the
//! duration expires, the game auto-pauses and never resumes -- and whatever the
//! worker is still holding at that moment is the last thing the stream will ever
//! be offered. There is no next iteration to re-attempt it, no later pass to
//! roll a reservation back to, and no other artifact that carries it. So the
//! terminal path has exactly two honest endings for every record it holds: the
//! record reached every authoritative sink, or the run says which record did
//! not and why.
//!
//! It had four ways of doing neither.
//!
//!   1. The terminal H4DG loop ended on `_ => break`. That wildcard covered a
//!      partial sink write, a committed campaign rollover, and an identity that
//!      would not settle. The reservation dropped uncommitted -- "rolled back to
//!      the scheduler", which is torn down a few statements later -- and every
//!      reserved domain-gap range went with it, unpublished and unmentioned.
//!   2. The terminal DATA write was an `if let ... = Written(outcome)` with no
//!      else, so both NOT-WRITTEN verdicts fell out of the bottom in silence.
//!      Only the Windows D75 custody branch noticed; every capture without that
//!      custody reported success with its last assembled hour never written.
//!   3. `RetryUncertainBoundary` means NOTHING happened -- not the identity, not
//!      the logical stream, not the published cursor -- so the record is still
//!      owed and re-attempting it costs nothing and loses nothing. The terminal
//!      path re-attempted zero times and dropped the frame instead.
//!   4. The two scheduler `RolledOver` arms invoked a reset macro whose first
//!      act is to refuse while the scheduler runtime is live -- which it is, by
//!      construction, on every path that reaches those writes. They read as
//!      "reset and continue" and were an abort wearing recovery's comment.
//!
//! Every test here fails if a terminal record can be lost without the run
//! saying so, if a shutdown re-attempt is unbounded, or if an exhausted
//! re-attempt reports a success it did not get.

use super::*;

/// The production source, read once. The shutdown sequence lives inside
//  `write_attach_heartbeat`, a single function of several thousand lines with no
/// seam a unit test can drive: it owns a live transport, a hook registry, an
/// observer thread and a process-global sweep service. The properties asserted
/// against this text are the ones that are structural -- which arms exist, what
/// each one does with the outcome it was handed -- and each is paired with a
/// demonstrated mutation in `terminal_write_path_mutations.py`.
const COLLECTOR_SOURCE: &str = include_str!("../attach_log.rs");

/// The terminal H4DG publication, comments stripped: from the reservation that
/// opens it to the non-compact fallback that follows it.
///
/// Stripped because these arms explain themselves at length and several QUOTE
/// the construct they replaced. Against the raw text, "the rollover arm appends
/// a typed error" would be satisfied by a comment SAYING so.
fn terminal_gap_region() -> String {
    code_only(
        COLLECTOR_SOURCE
            .split_once("let mut final_gap_reservation =")
            .expect("terminal gap publication")
            .1
            .split_once("} else if control.stream_format != CollectorStreamFormat::Compact {")
            .expect("terminal gap publication terminator")
            .0,
    )
}

/// The terminal DATA publication, comments stripped: from the bounded-retry call
/// that performs it to the incomplete-frame branch that follows the match.
fn terminal_data_region() -> String {
    code_only(
        COLLECTOR_SOURCE
            .split_once("let terminal_write_attempt =")
            .expect("terminal DATA publication")
            .1
            .split_once("} else if !frame_complete && terminal_control_error.is_none() {")
            .expect("terminal DATA publication terminator")
            .0,
    )
}

/// The same text with every `//` line comment removed.
///
/// These arms explain themselves at length, and several of them QUOTE the
/// construct they replaced. A structural assertion that read the comments would
/// be answering the prose instead of the code.
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

/// A write that every requested sink took.
///
/// Sealed through the production mint, because that is now the only way a
/// `Written` verdict exists at all. The scripted sink is handed no bytes, so
/// the sealed value carries no receipt -- which is correct, and is not what
/// these tests are about: they assert what the bounded re-attempt DOES with
/// each verdict. The receipt's own rules are pinned in `write_receipt_tests`.
fn complete_outcome() -> WrittenFrame {
    seal_completed_write(
        SinkWriteOutcome {
            file_requested: true,
            file_complete: true,
            pipe_requested: false,
            pipe_complete: false,
        },
        b"",
    )
    .expect("a sealed complete write")
}

fn incomplete_outcome() -> WrittenFrame {
    seal_completed_write(
        SinkWriteOutcome {
            file_requested: true,
            file_complete: false,
            pipe_requested: false,
            pipe_complete: false,
        },
        b"",
    )
    .expect("a sealed incomplete write")
}

/// What the scripted sink does once its script runs out.
enum ScriptEnd {
    /// Keep answering "uncertain" forever. This is the live case the bound
    /// exists for: a campaign-identity read that will not settle does not
    /// eventually settle just because shutdown would like it to.
    ///
    /// Paired with `Refuse` below rather than used alone, because a helper that
    /// ignored its bound would spin here and HANG the suite instead of failing
    /// it, and a test that hangs proves nothing.
    KeepUncertain { budget: u32 },
}

/// A terminal sink that answers from an exact script and counts what it was
/// asked.
///
/// `RetryUncertainBoundary` is produced only by an `IdentityV1` transport whose
/// live campaign-identity read disagreed with itself across two reads. No
/// file-backed or legacy transport can ever report it -- legacy sinks answer
/// `Stable` unconditionally -- so the verdict sequence the bounded re-attempt
/// exists to handle is reachable only by scripting it.
struct ScriptedTerminalSink {
    verdicts: std::collections::VecDeque<CompactFrameWrite>,
    end: ScriptEnd,
    calls: u32,
}

impl ScriptedTerminalSink {
    fn new(verdicts: Vec<CompactFrameWrite>, end: ScriptEnd) -> Self {
        Self {
            verdicts: verdicts.into(),
            end,
            calls: 0,
        }
    }
}

impl TerminalCompactFrameSink for ScriptedTerminalSink {
    fn attempt_terminal_compact_frame(
        &mut self,
        _records: &[u8],
    ) -> Result<CompactFrameWrite, String> {
        self.calls = self.calls.saturating_add(1);
        if let Some(verdict) = self.verdicts.pop_front() {
            return Ok(verdict);
        }
        match self.end {
            ScriptEnd::KeepUncertain { budget } => {
                if self.calls > budget {
                    // A bounded re-attempt never gets here. An unbounded one
                    // fails its test on this error instead of spinning forever.
                    return Err(format!(
                        "terminal re-attempt exceeded its bound: attempt {} of a {budget}-attempt \
                         budget",
                        self.calls
                    ));
                }
                Ok(CompactFrameWrite::RetryUncertainBoundary)
            }
        }
    }
}

/// A back-off of zero. The production back-off is asserted separately, at the
/// call sites, by `terminal_publication_spends_the_production_retry_budget`:
/// a test that actually slept would spend a real second proving arithmetic.
const NO_BACKOFF: Duration = Duration::from_millis(0);

#[test]
fn an_uncertain_terminal_boundary_is_re_attempted_rather_than_dropped() {
    let mut sink = ScriptedTerminalSink::new(
        vec![
            CompactFrameWrite::RetryUncertainBoundary,
            CompactFrameWrite::RetryUncertainBoundary,
            CompactFrameWrite::Written(complete_outcome()),
        ],
        ScriptEnd::KeepUncertain { budget: 8 },
    );
    let attempt = publish_terminal_compact_frame(&mut sink, b"frame", 8, NO_BACKOFF)
        .expect("scripted terminal publication");
    assert!(
        matches!(attempt.outcome, CompactFrameWrite::Written(outcome) if outcome.complete()),
        "an identity that settles on the third read publishes the frame the first two owed"
    );
    assert_eq!(
        attempt.attempts, 3,
        "the frame is re-attempted once per unsettled read and no more"
    );
    assert_eq!(sink.calls, 3);
}

#[test]
fn an_exhausted_terminal_re_attempt_reports_the_verdict_it_got_not_a_success() {
    const BOUND: u32 = 5;
    let mut sink = ScriptedTerminalSink::new(Vec::new(), ScriptEnd::KeepUncertain { budget: BOUND });
    let attempt = publish_terminal_compact_frame(&mut sink, b"frame", BOUND, NO_BACKOFF)
        .expect("a bounded re-attempt never exceeds its budget");
    assert!(
        matches!(attempt.outcome, CompactFrameWrite::RetryUncertainBoundary),
        "an identity that never settles is reported as never settled; nothing was written"
    );
    assert_eq!(
        attempt.attempts, BOUND,
        "the budget is spent exactly once through, not partly and not twice"
    );
    assert_eq!(
        sink.calls, BOUND,
        "shutdown stops asking after its budget: the game outlives this detach"
    );
}

#[test]
fn a_committed_terminal_rollover_is_not_re_attempted() {
    let mut sink = ScriptedTerminalSink::new(
        vec![CompactFrameWrite::RolledOver],
        ScriptEnd::KeepUncertain { budget: 1 },
    );
    let attempt = publish_terminal_compact_frame(&mut sink, b"frame", 20, NO_BACKOFF)
        .expect("scripted terminal publication");
    assert!(matches!(attempt.outcome, CompactFrameWrite::RolledOver));
    assert_eq!(
        attempt.attempts, 1,
        "the boundary was already committed; asking again only observes the same commit"
    );
}

#[test]
fn an_incomplete_terminal_sink_write_is_not_re_attempted() {
    let mut sink = ScriptedTerminalSink::new(
        vec![CompactFrameWrite::Written(incomplete_outcome())],
        ScriptEnd::KeepUncertain { budget: 1 },
    );
    let attempt = publish_terminal_compact_frame(&mut sink, b"frame", 20, NO_BACKOFF)
        .expect("scripted terminal publication");
    assert!(
        matches!(attempt.outcome, CompactFrameWrite::Written(outcome) if !outcome.complete()),
        "a refused write is reported refused"
    );
    assert_eq!(
        attempt.attempts, 1,
        "these bytes already reached the sinks that accepted them; repeating duplicates them"
    );
}

#[test]
fn a_terminal_publication_error_is_surfaced_rather_than_retried_away() {
    struct FailingSink;
    impl TerminalCompactFrameSink for FailingSink {
        fn attempt_terminal_compact_frame(
            &mut self,
            _records: &[u8],
        ) -> Result<CompactFrameWrite, String> {
            Err("typed identity failure was durably acknowledged over H4CT".into())
        }
    }
    let error = publish_terminal_compact_frame(&mut FailingSink, b"frame", 20, NO_BACKOFF)
        .expect_err("a transport error is not a verdict to retry");
    assert!(error.contains("typed identity failure"));
}

#[test]
fn terminal_publication_spends_the_production_retry_budget() {
    // ~1s at the capture loop's own 50ms back-off: long enough for an identity
    // read to settle, short enough that teardown still ends.
    assert_eq!(TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS, 20);
    assert!(
        TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS >= 2,
        "a bound of one is a zero-retry drop with extra steps"
    );
    // The DATA and H4DG publications each carry their own budget and run one
    // after the other, so the number that has to stay small is the SUM. (Each
    // sleeps between attempts, not after the last, so it is one fewer back-off
    // than attempts -- counted generously here.)
    let worst_case = CAMPAIGN_IDENTITY_UNCERTAIN_BOUNDARY_BACKOFF
        * TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS
        * 2;
    assert!(
        worst_case <= Duration::from_millis(2_500),
        "shutdown must end: the terminal DATA and H4DG budgets together add {worst_case:?}"
    );

    // Both terminal publications spend that budget, and no other one.
    let data = terminal_data_region();
    assert_eq!(
        data.matches("publish_terminal_compact_frame(").count(),
        2,
        "BOTH terminal DATA branches re-attempt an unsettled identity: the Windows one that has \
         no D75 custody to publish through, and the one compiled everywhere else"
    );
    assert!(
        data.contains("TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS")
            && data.contains("CAMPAIGN_IDENTITY_UNCERTAIN_BOUNDARY_BACKOFF"),
        "the terminal DATA write spends the production budget, not one invented here"
    );
    let gaps = &terminal_gap_region();
    assert!(
        gaps.contains("uncertain_gap_attempts >= TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS")
            && gaps.contains("thread::sleep(CAMPAIGN_IDENTITY_UNCERTAIN_BOUNDARY_BACKOFF)"),
        "the terminal H4DG write re-attempts an unsettled identity on the production budget"
    );
    assert_eq!(
        gaps.matches("let mut uncertain_gap_attempts").count(),
        1,
        "there is one re-attempt budget for the terminal H4DG publication"
    );
    assert!(
        gaps.find("let mut uncertain_gap_attempts").expect("H4DG budget")
            < gaps
                .find("while !final_gap_reservation.gaps().is_empty()")
                .expect("H4DG publication loop"),
        "the budget is declared OUTSIDE the group loop: one inside it resets per group and \
         multiplies the wait by the number of reserved groups, which is the hang this bound \
         exists to prevent"
    );
}

#[test]
fn the_terminal_gap_publication_names_every_range_it_could_not_publish() {
    let gaps = terminal_gap_region();
    assert!(
        !gaps.contains("_ => {"),
        "a wildcard arm here is how three different endings became one silent exit"
    );
    for variant in [
        "CompactFrameWrite::Written(outcome) if outcome.complete()",
        "CompactFrameWrite::Written(_)",
        "CompactFrameWrite::RolledOver",
        "CompactFrameWrite::RetryUncertainBoundary",
    ] {
        assert!(
            gaps.contains(variant),
            "the terminal H4DG write must answer {variant} explicitly"
        );
    }
    // Every ending that is not "published" states the cost, and states it in
    // reserved ranges: a reservation that drops uncommitted at shutdown is not
    // rolled back to anything, because nothing runs after it.
    assert_eq!(
        gaps.matches("final_gap_reservation.gaps().len()").count(),
        5,
        "each unpublished ending counts the ranges it lost: the three write verdicts, the sink \
         that was already terminal before publication began, and the successor stream"
    );
    assert_eq!(
        gaps.matches("append_terminal_error(").count(),
        5,
        "and each one fails the run rather than ending it quietly"
    );
    // The successor-stream guard. A rollover committed during the terminal DATA
    // publication opens a new logical stream; these ranges describe the closed
    // one and are numbered from a cursor that counts the closed one's records,
    // so publishing them here would stamp old-stream numbers onto a stream that
    // starts at sequence one.
    assert!(
        gaps.find("transport.current_logical_stream_id() != terminal_logical_stream")
            .expect("successor-stream guard")
            < gaps
                .find("while !final_gap_reservation.gaps().is_empty()")
                .expect("H4DG publication loop"),
        "the successor-stream check gates the publication loop; a check after it would run \
         once the ranges were already on the wrong stream"
    );
    // A stream-id latch taken at the start of the terminal sequence can only see
    // a rollover committed AFTER it. The capture loop's own scheduler writes can
    // commit one and then break into that sequence, at which point the successor
    // already IS the current stream and the latch compares equal. The flag is
    // what covers that window, and it must gate the same publication.
    assert!(
        gaps.contains("scheduler_stream_closed_by_write"),
        "a rollover committed by a write BEFORE the terminal sequence began also stops H4DG \
         publication; the stream-id latch cannot see that one"
    );
    let capture_loop = COLLECTOR_SOURCE
        .split_once("'capture: while Instant::now() <= deadline {")
        .expect("capture loop");
    assert!(
        capture_loop
            .0
            .contains("let mut scheduler_stream_closed_by_write = false;"),
        "the flag is declared outside the loop, so a write that closes the stream is still \
         known once the loop has broken"
    );
    assert_eq!(
        code_only(capture_loop.1)
            .matches("scheduler_stream_closed_by_write = true")
            .count(),
        3,
        "every site that reaches teardown after a write committed a rollover sets it: the \
         scheduler DATA publication, the frame-refusal publication, and the terminal DATA \
         write"
    );
    // THREE, not four. The scheduler H4DG write used to be the fourth, because
    // it called `write_compact_frame` and so took its own campaign-identity
    // observation, which could commit a rollover of its own.
    //
    // It no longer observes anything. The gap frames are staged behind the DATA
    // frame and published under the one boundary result the DATA publication
    // already admitted, through `write_preboundary_compact_frame` -- which is
    // exactly the API for "records assembled for the currently admitted
    // identity", and which has no rollover ending to reach. A site that cannot
    // commit a rollover has no flag to set, so a fourth assignment here would
    // be dead code claiming to guard a window that closed.
    let gap_publication = capture_loop
        .1
        .split_once("if let Some(runtime) = read_only_scheduler_runtime.as_ref() {")
        .expect("scheduler control-gap block")
        .1
        .split_once("// THE OVERLAP")
        .expect("scheduler control-gap block terminator")
        .0;
    let gap_publication = code_only(gap_publication);
    assert!(
        gap_publication.contains("StagedPublicationKind::ControlGap"),
        "the H4DG frames are staged for publication, not written inline"
    );
    assert!(
        !gap_publication.contains("write_compact_frame"),
        "an inline `write_compact_frame` here would take a SECOND identity observation and \
         could commit a rollover that nothing above records -- which is the fourth site this \
         count deliberately no longer has"
    );
    // The refusal publication reports a committed rollover and an identity that
    // never settled through different channels, and only the first closes the
    // stream. If that verdict ever came to mean both, the flag above would fire
    // on a stream that is still open and suppress H4DG that could have been
    // published.
    let refusal = COLLECTOR_SOURCE
        .split_once("fn publish_frame_refusal_record(")
        .expect("frame refusal publication")
        .1
        .split_once("mod frame_refusal_sequence_tests;")
        .expect("frame refusal publication terminator")
        .0;
    let refusal = code_only(refusal);
    assert_eq!(
        refusal
            .matches("FrameRefusalPublication::NotPublishedAtIdentityBoundary")
            .count(),
        1,
        "the verdict has exactly one producer"
    );
    let rollover_arm = refusal
        .split_once("CompactFrameWrite::RolledOver => {")
        .expect("refusal rollover arm")
        .1;
    assert!(
        rollover_arm
            .trim_start()
            .starts_with("return Ok(FrameRefusalPublication::NotPublishedAtIdentityBoundary)"),
        "and that producer is the committed-rollover arm, not the uncertain one"
    );
    assert!(
        refusal.contains("stayed uncertain across"),
        "an identity that never settles is an Err, so the caller cannot mistake it for a \
         closed stream"
    );
    assert!(
        !gaps.contains("terminal_control_error = Some(match"),
        "the cause is appended, never rebuilt by hand around a take()"
    );
}

#[test]
fn the_terminal_data_write_answers_every_not_written_outcome() {
    let data = terminal_data_region();
    assert!(
        !data.contains("if let CompactFrameWrite::Written(outcome) = terminal_write_outcome"),
        "an `if let` with no else is how both not-written verdicts fell out in silence"
    );
    assert!(
        data.contains("match terminal_write_outcome {"),
        "the terminal DATA outcome is matched exhaustively"
    );
    assert!(
        data.contains("CompactFrameWrite::Written(outcome) if outcome.complete() =>"),
        "only a write that reached EVERY authoritative sink commits the staged cursor"
    );
    let published = data
        .split_once("CompactFrameWrite::Written(outcome) if outcome.complete() =>")
        .expect("published arm")
        .1;
    let (published, not_published) = published
        .split_once("not_published =>")
        .expect("the not-written arm");
    // `commit_published_frame` and not `commit_staged(next_sequence)`: the
    // cursor is moved by the WRITE's receipt, and `outcome` -- the sealed write
    // -- is what carries it. The arm has to give up the outcome to commit, so
    // the two cannot be separated by an edit that keeps compiling.
    assert!(
        published.contains("commit_published_frame(")
            && published.contains("outcome,")
            && published.contains("next_sequence,"),
        "the published arm is the only one that moves the published cursor, and it moves it with \
         the write that reached the sinks"
    );
    assert!(
        !not_published.contains("commit_staged")
            && !not_published.contains("commit_published_frame")
            && !not_published.contains("commit_all()")
            && !not_published.contains("record_complete_frame_emitted_to_sink"),
        "a frame that was not written commits nothing and is not reported to the scheduler as \
         emitted"
    );
    assert!(
        not_published.contains("append_terminal_error("),
        "a terminal DATA frame that was not durably published fails the run"
    );
    for verdict in [
        "CompactFrameWrite::Written(_)",
        "CompactFrameWrite::RolledOver",
        "CompactFrameWrite::RetryUncertainBoundary",
    ] {
        assert!(
            not_published.contains(verdict),
            "the failure names WHICH ending it met: {verdict}"
        );
    }
    assert!(
        not_published.contains("terminal_write_attempts"),
        "\"the identity stayed uncertain\" and \"the identity was never read\" are different \
         failures, and the attempt count is what tells them apart"
    );
}

#[test]
fn a_rollover_committed_inside_a_scheduler_write_stops_rather_than_pretending_to_reset() {
    // The reset macro's own guard, unchanged: it refuses while the old
    // scheduler still holds DATA-or-H4DG custody.
    let reset_body = COLLECTOR_SOURCE
        .split_once("macro_rules! reset_campaign_scoped_capture_state")
        .expect("campaign reset macro")
        .1;
    assert!(
        reset_body
            .split_once("// A28's hook owners retain")
            .expect("campaign reset scheduler terminator")
            .0
            .contains("read_only_scheduler_runtime.is_some()"),
        "the guard this lane is about: a live scheduler runtime is unfinalized custody"
    );

    // The two scheduler writes are reachable only while that runtime is live,
    // so a reset invoked from their rollover arms could only ever resolve to the
    // guard. Neither arm may invoke it.
    let capture_loop = COLLECTOR_SOURCE
        .split_once("'capture: while Instant::now() <= deadline {")
        .expect("capture loop")
        .1;
    // ONE scheduler site observes a boundary now, not two. The H4DG frames are
    // staged behind the DATA frame and published under the boundary result that
    // site already admitted, so there is no second observation to roll over.
    assert!(
        capture_loop.contains("scheduler DATA publication for hour {pending_hour}")
            && capture_loop.contains("closing the logical stream while the old scheduler still"),
        "the scheduler DATA publication states the committed rollover as its terminal cause"
    );

    let (data_rollover, rest) = capture_loop
        .split_once("PreboundaryPublication::RolledOver => {")
        .expect("scheduler DATA rollover arm")
        .1
        .split_once("// RELEASE READ CUSTODY")
        .expect("scheduler DATA rollover arm terminator");
    let data_rollover = code_only(data_rollover);
    assert!(
        !data_rollover.contains("reset_campaign_scoped_capture_state!")
            && data_rollover.contains("break 'capture"),
        "the scheduler DATA rollover arm aborts and says so; it does not invoke a reset that \
         cannot run"
    );
    assert!(
        data_rollover.contains("scheduler_stream_closed_by_write = true"),
        "and it records that the stream is closed, so the terminal H4DG publication does not \
         put old-stream ranges on the successor"
    );

    // The legacy (scheduler-off) write still resets and continues, because
    // there is no scheduler runtime to hold custody and the guard passes.
    let legacy_rollover = rest
        .split_once("if records.is_empty() {")
        .expect("legacy compact write")
        .1;
    assert!(
        legacy_rollover.contains("reset_campaign_scoped_capture_state!('capture)"),
        "the scheduler-off write has no custody to lose and still recovers across a rollover"
    );
}

/// THE STAGING SLOT IS EMPTIED ON BOTH ENDINGS, AND ONCE.
///
/// `staged_publications` outlives the loop body so a `break` between staging a
/// frame and publishing it can still be flushed below the loop. That lifetime
/// is what makes emptying it load-bearing, in two opposite ways:
///
///   * not cleared after a SUCCESSFUL resolve, the next sweep's own guard sees
///     this hour's published frames sitting in the slot and reads them as last
///     hour's unpublished ones -- every capture would abort on its second sweep,
///     and no offline test can reach a second sweep because that needs a live
///     game clock;
///   * not cleared after a FAILED resolve, the post-loop flush re-offers buffers
///     the publisher already wrote, putting duplicate records on the stream.
///
/// So the clear must sit between the resolve and the `break`, not inside either
/// arm. This checks exactly that placement.
///
/// Mutation proof: move `staged_publications.clear()` above the resolve call and
/// the borrow no longer compiles; move it after the `break 'capture` and the
/// ordering assertion below fails; delete it and the "cleared exactly once"
/// count fails.
#[test]
fn a_resolved_publication_slot_is_emptied_before_the_loop_can_break() {
    let capture_loop = COLLECTOR_SOURCE
        .split_once("'capture: while Instant::now() <= deadline {")
        .expect("capture loop")
        .1;
    let resolve_block = capture_loop
        .split_once("let resolution = resolve_staged_publications(")
        .expect("the in-loop resolution")
        .1
        .split_once("if let Some(reservation) = deferred_gap_reservation.take()")
        .expect("resolution block terminator")
        .0;
    let resolve_block = code_only(resolve_block);
    let clear_at = resolve_block
        .find("staged_publications.clear();")
        .expect("the slot is emptied after the resolve");
    let break_at = resolve_block
        .find("break 'capture;")
        .expect("a failed resolution ends the capture");
    assert!(
        clear_at < break_at,
        "the clear must run on the failure path too; after the `break` it never runs, and the \
         post-loop flush would re-offer buffers the publisher already wrote"
    );
    assert_eq!(
        resolve_block.matches("staged_publications.clear();").count(),
        1,
        "cleared once, unconditionally -- a clear inside either match arm covers one ending only"
    );
    assert!(
        !resolve_block.contains("Ok(()) => {}"),
        "the clear is unconditional, so the resolution is not matched into per-arm blocks that \
         could each forget it"
    );
}

#[cfg(windows)]
#[test]
fn the_d75_terminal_writer_refuses_an_uncertain_boundary_instead_of_claiming_a_write() {
    // The one terminal publication that does NOT re-attempt an uncertain
    // boundary, and the reason is outside this crate: every error out of this
    // `Write` is classified by `publish_terminal_h4cr_to_authoritative_sink` as
    // `retry_permit: None, sink_state_indeterminate: true`, so the one retry
    // permit that API already carries cannot be reached by a refusal that
    // provably wrote nothing.
    //
    // That is a residual, and it is fail-closed: the caller aborts the D75
    // custody and fails the run. What this test forbids is the other ending --
    // the writer reporting bytes accepted, or falling through to
    // `accepted_outcome`, when the transport told it nothing was written.
    let writer = COLLECTOR_SOURCE
        .split_once("impl Write for D75TerminalAuthoritativeWriter<'_> {")
        .expect("D75 terminal writer")
        .1
        .split_once("fn flush(&mut self)")
        .expect("D75 terminal writer terminator")
        .0;
    let writer = code_only(writer);
    for (variant, kind) in [
        ("CompactFrameWrite::RolledOver", "ErrorKind::WouldBlock"),
        (
            "CompactFrameWrite::RetryUncertainBoundary",
            "ErrorKind::WouldBlock",
        ),
    ] {
        let arm = writer
            .split_once(&format!("{variant} => {{"))
            .unwrap_or_else(|| panic!("D75 writer arm for {variant}"))
            .1
            .split_once('}')
            .expect("D75 writer arm terminator")
            .0;
        assert!(
            arm.contains(kind) && arm.contains("return Err("),
            "{variant} wrote nothing, so the writer refuses with a typed {kind}"
        );
        assert!(
            !arm.contains("accepted_outcome"),
            "{variant} must never reach the accepted-outcome path"
        );
    }
    assert!(
        writer.contains("if !outcome.complete()"),
        "a write that reached only some authoritative sinks is refused too"
    );

    // And the caller does not shrug it off. A refusal out of that writer reaches
    // `custody.publish` as an `Err`, and the only thing the terminal DATA site
    // may do with it is release the custody and fail the run -- never fall
    // through to the published arm, and never leave the reservation held.
    let data = terminal_data_region();
    let custody_failure = data
        .split_once("match custody.publish(&mut transport, &records) {")
        .expect("D75 custody publication")
        .1
        .split_once("} else {")
        .expect("D75 custody publication terminator")
        .0;
    assert!(
        custody_failure.contains("Err(error) => {")
            && custody_failure.contains("abort_terminal_d75_owner(")
            && custody_failure.contains("return Err("),
        "a D75 terminal publication failure aborts the custody and fails the worker"
    );
    assert!(
        !custody_failure.contains("commit_staged")
            && !custody_failure.contains("commit_published_frame")
            && !custody_failure.contains("custody.commit()"),
        "nothing is committed on the path where the terminal H4CR was not accepted"
    );
}

#[cfg(windows)]
#[test]
fn the_attach_worker_joins_the_sweep_thread_on_every_capture_outcome() {
    let worker = COLLECTOR_SOURCE
        .split_once("unsafe extern \"system\" fn attach_worker(")
        .expect("attach worker")
        .1
        .split_once("/// Whether this module may still be unloaded.")
        .expect("attach worker terminator")
        .0;
    let capture = worker
        .find("let capture_result = write_attach_heartbeat(")
        .expect("capture call");
    let join = worker
        .find("heap_sweep_shutdown();")
        .expect("the sweep thread is joined in the worker that unloads the module");
    assert!(
        join > capture,
        "the join happens after the capture, so it covers every way the capture can end"
    );
    // The LAST unload, deliberately. The worker has an earlier one on the
    // startup-failure path, and that path returns before the capture runs --
    // every `heap_sweep_ensure_started` call site is inside the capture, so
    // there is no thread to join there and nothing for this ordering to say.
    let unload = worker
        .rfind("FreeLibraryAndExitThread(")
        .expect("module unload");
    assert!(
        join < unload,
        "the sweep thread must be joined BEFORE the module can be unloaded from under it"
    );
    let between = code_only(&worker[capture..join]);
    assert!(
        !between.contains("return ") && !between.contains("?;"),
        "nothing between the capture and the join may exit first"
    );
}
