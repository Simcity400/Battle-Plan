use super::*;

const COLLECTOR_SOURCE: &str = include_str!("../attach_log.rs");

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
fn every_production_rollover_exit_routes_through_successor_census_recovery() {
    let production = COLLECTOR_SOURCE
        .split_once("mod freeze_census_publication_tests")
        .expect("freeze census publication test module")
        .0;
    assert_eq!(
        production
            .matches("require_successor_install_census_after_rollover(")
            .count(),
        8,
        "seven terminal/boundary call sites plus the function definition are required"
    );
    assert_eq!(
        production
            .matches("publish_successor_install_census_after_rollover(")
            .count(),
        5,
        "campaign reset, the initial-H4CI rollover retry, the initial-H4SC rollover retry,          the function definition and the required-wrapper delegation are required"
    );
    let freeze_arguments = production
        .matches("freeze_census_payload.as_deref(),")
        .count();
    let identity_arguments = production
        .matches("content_identity_payload.as_deref(),")
        .count();
    let topology_arguments = production
        .matches("state_topology_payload.as_deref(),")
        .count();
    assert_eq!(
        identity_arguments,
        freeze_arguments - 1,
        "every rollover site that passes the install freeze census must pass the retained          content identity census with it; the one exception is the initial H4CI publication,          which IS the census and carries only the freeze payload"
    );
    assert_eq!(
        topology_arguments,
        identity_arguments - 1,
        "and the retained state-topology census rides with them, with the same one          exception one step further along: the initial H4SC publication IS that census,          so it carries the freeze and identity payloads but not itself"
    );
    assert!(
        freeze_arguments >= 9,
        "the rollover sites are named individually above; this is the floor they imply"
    );
}

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

enum ScriptEnd {
    KeepUncertain { budget: u32 },
}

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
    let mut sink =
        ScriptedTerminalSink::new(Vec::new(), ScriptEnd::KeepUncertain { budget: BOUND });
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
    assert_eq!(TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS, 20);
    assert!(
        TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS >= 2,
        "a bound of one is a zero-retry drop with extra steps"
    );
    let worst_case =
        CAMPAIGN_IDENTITY_UNCERTAIN_BOUNDARY_BACKOFF * TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS * 2;
    assert!(
        worst_case <= Duration::from_millis(2_500),
        "shutdown must end: the terminal DATA and H4DG budgets together add {worst_case:?}"
    );

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
        gaps.find("let mut uncertain_gap_attempts")
            .expect("H4DG budget")
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
    assert_eq!(
        gaps.matches("final_gap_reservation.gaps().len()").count(),
        5,
        "each unpublished ending counts the ranges it lost: the three write verdicts, the sink \
         that was already terminal before publication began, and the successor stream"
    );
    assert_eq!(
        gaps.matches("append_terminal_error(").count(),
        6,
        "and each one fails the run rather than ending it quietly, including a successor whose cached census cannot be published"
    );
    assert!(
        gaps.find("transport.current_logical_stream_id() != terminal_logical_stream")
            .expect("successor-stream guard")
            < gaps
                .find("while !final_gap_reservation.gaps().is_empty()")
                .expect("H4DG publication loop"),
        "the successor-stream check gates the publication loop; a check after it would run \
         once the ranges were already on the wrong stream"
    );
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
        6,
        "every site that reaches teardown after a write committed a rollover sets it: the \
         scheduler DATA publication, the frame-refusal publication, the clock-unreadable \
         interval publication, the untrusted clock-recovery publication, the ordinary terminal \
         DATA write, and the D75-owned terminal DATA write"
    );
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

    let capture_loop = COLLECTOR_SOURCE
        .split_once("'capture: while Instant::now() <= deadline {")
        .expect("capture loop")
        .1;
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

    let legacy_rollover = rest
        .split_once("if records.is_empty() {")
        .expect("legacy compact write")
        .1;
    assert!(
        legacy_rollover.contains("reset_campaign_scoped_capture_state!('capture)"),
        "the scheduler-off write has no custody to lose and still recovers across a rollover"
    );
}

/// frame and publishing it can still be flushed below the loop. That lifetime
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
        resolve_block
            .matches("staged_publications.clear();")
            .count(),
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

    // may do with it is release the custody and fail the run -- never fall
    let data = terminal_data_region();
    let custody_failure = data
        .split_once("match custody.publish(&mut transport, &records) {")
        .expect("D75 custody publication")
        .1
        .split_once("} else {")
        .expect("D75 custody publication terminator")
        .0;
    assert!(
        custody_failure.contains("Err(mut error) => {")
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
