"""Mutation battery for lane/terminal_write_path.

Each entry mutates PRODUCTION code and names the test that must go red.
A mutation whose named test stays green is a SURVIVOR and is reported.

Run from the worktree root. Reverts the touched file after every mutation, via
`git checkout --`, so COMMIT before running: uncommitted production edits are
destroyed.

    python terminal_write_path_mutations.py            # whole battery
    python terminal_write_path_mutations.py T3 T7      # named entries only

Evidence lands in extractor/native/runs/terminal_write_path/.
"""

import subprocess
import sys
import os

# The worktree this battery is run from, derived from the file's own
# location. It used to be a hardcoded lane path, which meant running it from
# any other worktree mutated -- and `git checkout --` reverted -- files in a
# tree the runner was not looking at, or failed outright once that lane was
# retired.
ROOT = os.path.dirname(os.path.abspath(__file__))
NATIVE = os.path.join(ROOT, "extractor", "native")
ATTACH = os.path.join(NATIVE, "collector", "src", "attach_log.rs")
TESTMOD = "attach_log::terminal_write_path_tests"

ENV = dict(
    os.environ,
    CARGO_TARGET_DIR=os.path.join(ROOT, ".cargo-target"),
    RUSTC_WRAPPER="sccache",
    CARGO_INCREMENTAL="0",
)

# (label, file, old, new, expected_red_test_or_None_for_compile_error)
MUTATIONS = [
    (
        "T1 the terminal H4DG loop drops every ending but the published one "
        "(the shipped `_ => break`)",
        ATTACH,
        """                            CompactFrameWrite::Written(_) => {
                                append_terminal_error(
                                    &mut terminal_control_error,
                                    format!(
                                        "terminal scheduler H4DG frame did not reach every \\
                                         authoritative sink; {} reserved domain-gap range(s) \\
                                         were not published",
                                        final_gap_reservation.gaps().len()
                                    ),
                                );
                                break;
                            }""",
        """                            _ => {
                                break;
                            }""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T2 a lost H4DG range is counted but not surfaced (diagnostic without a verdict)",
        ATTACH,
        """                                append_terminal_error(
                                    &mut terminal_control_error,
                                    format!(
                                        "a campaign rollover closed the logical stream during \\
                                         terminal H4DG publication; {} reserved domain-gap \\
                                         range(s) belonged to the closed stream and were not \\
                                         published",
                                        final_gap_reservation.gaps().len()
                                    ),
                                );
                                break;""",
        """                                break;""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T3 the terminal DATA write ignores both not-written verdicts again",
        ATTACH,
        """                            match terminal_write_outcome {
                                CompactFrameWrite::Written(outcome) if outcome.complete() => {""",
        """                            if let CompactFrameWrite::Written(outcome) = terminal_write_outcome {
                                if outcome.complete() {""",
        None,  # must not compile: the not_published arm loses its match
    ),
    (
        "T4 the terminal DATA loss is recorded without saying which ending it met",
        ATTACH,
        """                                    let verdict = match not_published {
                                        CompactFrameWrite::Written(_) => {
                                            "did not reach every authoritative sink"
                                        }
                                        CompactFrameWrite::RolledOver => {
                                            scheduler_stream_closed_by_write = true;
                                            "met a campaign rollover that closed the logical \\
                                             stream it belonged to before it was written"
                                        }
                                        CompactFrameWrite::RetryUncertainBoundary => {
                                            "met a campaign identity that would not settle, so \\
                                             nothing was written"
                                        }
                                    };""",
        """                                    let _ = &not_published;
                                    let verdict = "was not written";""",
        "the_terminal_data_write_answers_every_not_written_outcome",
    ),
    (
        "T5 an uncertain terminal boundary is dropped instead of re-attempted "
        "(the zero-retry discard)",
        ATTACH,
        """        if !matches!(outcome, CompactFrameWrite::RetryUncertainBoundary)
            || attempts >= attempt_bound
        {
            return Ok(TerminalFrameAttempt { outcome, attempts });
        }
        thread::sleep(backoff);""",
        """        let _ = (attempt_bound, backoff);
        return Ok(TerminalFrameAttempt { outcome, attempts });""",
        "an_uncertain_terminal_boundary_is_re_attempted_rather_than_dropped",
    ),
    (
        "T6 the terminal re-attempt ignores its bound (teardown can hang)",
        ATTACH,
        """        if !matches!(outcome, CompactFrameWrite::RetryUncertainBoundary)
            || attempts >= attempt_bound
        {""",
        """        if !matches!(outcome, CompactFrameWrite::RetryUncertainBoundary) {""",
        "an_exhausted_terminal_re_attempt_reports_the_verdict_it_got_not_a_success",
    ),
    (
        "T7 an exhausted terminal re-attempt reports a success it never got",
        ATTACH,
        # MUTANT REPAIRED by lane/write_receipt: `Written` carries a sealed
        # write now, so fabricating a success has to go through the mint. The
        # fabrication is still exactly as available as it was -- which is the
        # point of keeping this entry: the receipt does not stop a shutdown from
        # LYING about its verdict, only from committing the cursor on one.
        """            return Ok(TerminalFrameAttempt { outcome, attempts });
        }
        thread::sleep(backoff);""",
        """            return Ok(TerminalFrameAttempt {
                outcome: CompactFrameWrite::Written(seal_completed_write(
                    SinkWriteOutcome {
                        file_requested: true,
                        file_complete: true,
                        pipe_requested: false,
                        pipe_complete: false,
                    },
                    records,
                )?),
                attempts,
            });
        }
        thread::sleep(backoff);""",
        "an_exhausted_terminal_re_attempt_reports_the_verdict_it_got_not_a_success",
    ),
    (
        "T8 a committed rollover is re-attempted like an uncertain boundary",
        ATTACH,
        """        if !matches!(outcome, CompactFrameWrite::RetryUncertainBoundary)
            || attempts >= attempt_bound
        {""",
        """        if matches!(outcome, CompactFrameWrite::Written(_)) || attempts >= attempt_bound {""",
        "a_committed_terminal_rollover_is_not_re_attempted",
    ),
    (
        "T9 the terminal H4DG re-attempt gets a per-group budget instead of one budget",
        ATTACH,
        """                    let mut uncertain_gap_attempts: u32 = 0;
                    while !final_gap_reservation.gaps().is_empty() {""",
        """                    while !final_gap_reservation.gaps().is_empty() {
                        let mut uncertain_gap_attempts: u32 = 0;""",
        "terminal_publication_spends_the_production_retry_budget",
    ),
    (
        "T10 the terminal DATA write stops re-attempting an unsettled identity",
        ATTACH,
        """                                    publish_terminal_compact_frame(
                                        &mut transport,
                                        &records,
                                        TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS,
                                        CAMPAIGN_IDENTITY_UNCERTAIN_BOUNDARY_BACKOFF,
                                    )?""",
        """                                    TerminalFrameAttempt {
                                        outcome: transport.write_compact_frame(&records)?,
                                        attempts: 1,
                                    }""",
        "terminal_publication_spends_the_production_retry_budget",
    ),
    (
        "T11 the terminal re-attempt budget is raised past what teardown can afford",
        ATTACH,
        """const TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS: u32 = 20;""",
        """const TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS: u32 = 2_000;""",
        "terminal_publication_spends_the_production_retry_budget",
    ),
    (
        "T12 the scheduler DATA rollover arm goes back to pretending it can reset",
        ATTACH,
        """                                append_terminal_error(
                                    &mut terminal_control_error,
                                    format!(
                                        "a campaign rollover was committed inside the scheduler \\
                                         DATA write for hour {pending_hour}, closing the logical \\
                                         stream while the old scheduler still held \\
                                         DATA-or-H4DG custody for it"
                                    ),
                                );
                                break 'capture;""",
        """                                reset_campaign_scoped_capture_state!('capture);
                                continue 'capture;""",
        "a_rollover_committed_inside_a_scheduler_write_stops_rather_than_pretending_to_reset",
    ),
    (
        "T13 the scheduler H4DG rollover arm goes back to pretending it can reset",
        ATTACH,
        """                                        append_terminal_error(
                                            &mut terminal_control_error,
                                            format!(
                                                "a campaign rollover was committed inside the \\
                                                 scheduler H4DG write, closing the logical \\
                                                 stream that {} reserved domain-gap range(s) \\
                                                 described",
                                                gap_reservation.gaps().len()
                                            ),
                                        );
                                        break 'capture;""",
        """                                        reset_campaign_scoped_capture_state!('capture);
                                        continue 'capture;""",
        "a_rollover_committed_inside_a_scheduler_write_stops_rather_than_pretending_to_reset",
    ),
    (
        "T14 the unconditional sweep join is removed from the attach worker",
        ATTACH,
        """    heap_sweep_shutdown();
    // Hooks remain installed until terminal purge telemetry has been""",
        """    // Hooks remain installed until terminal purge telemetry has been""",
        "the_attach_worker_joins_the_sweep_thread_on_every_capture_outcome",
    ),
    (
        # UPGRADED by lane/write_receipt from a test-kill to a COMPILE-PROOF.
        # This entry used to compile and be caught by a structural assertion.
        # `commit_staged` now consumes a write receipt, and `not_published` is
        # bound to the two verdicts that carry no value at all, so the mutation
        # this battery was written to catch no longer type-checks.
        "T15 a not-written terminal DATA frame commits the published cursor anyway",
        ATTACH,
        """                                not_published => {
                                    let verdict = match not_published {""",
        """                                not_published => {
                                    session_sequence.commit_staged(next_sequence)?;
                                    let verdict = match not_published {""",
        None,  # must not compile
    ),
    (
        # T3 kills by refusing to compile, which proves the shape is gone but not
        # that the property is watched. This one COMPILES: the match stays
        # exhaustive and the not-written arm silently does nothing, which is the
        # shipped defect expressed in the current shape.
        #
        # An earlier version of this entry SHADOWED the real arm with an
        # unreachable `_not_published => {}` placed ahead of it, and it SURVIVED
        # -- correctly, and the survival is worth recording. A structural
        # assertion reads shape, not reachability, so it cannot see an arm that
        # is present but dead. What stops that one is not this battery: rustc's
        # `unreachable_patterns` warning fires on it, and the mutation had to add
        # `#[allow(unreachable_patterns)]` to compile quietly at all.
        "T16 the terminal DATA not-written arm compiles and says nothing",
        ATTACH,
        """                                not_published => {
                                    let verdict = match not_published {
                                        CompactFrameWrite::Written(_) => {
                                            "did not reach every authoritative sink"
                                        }
                                        CompactFrameWrite::RolledOver => {
                                            scheduler_stream_closed_by_write = true;
                                            "met a campaign rollover that closed the logical \\
                                             stream it belonged to before it was written"
                                        }
                                        CompactFrameWrite::RetryUncertainBoundary => {
                                            "met a campaign identity that would not settle, so \\
                                             nothing was written"
                                        }
                                    };
                                    append_terminal_error(
                                        &mut terminal_control_error,
                                        format!(
                                            "terminal read-only scheduler DATA frame for hour \\
                                             {pending_hour} {verdict} after \\
                                             {terminal_write_attempts} attempt(s) and was not \\
                                             durably published"
                                        ),
                                    );
                                }""",
        """                                _not_published => {}""",
        "the_terminal_data_write_answers_every_not_written_outcome",
    ),
    (
        "T17 terminal H4DG ranges are published into the successor stream after a "
        "rollover committed during terminal DATA",
        ATTACH,
        """                } else if scheduler_stream_closed_by_write
                    || transport.current_logical_stream_id() != terminal_logical_stream
                {""",
        """                } else if false {""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T18 the successor-stream loss is reported without counting the ranges it lost",
        ATTACH,
        """                            "a campaign rollover committed inside a compact write closed the \\
                             logical stream before terminal H4DG publication; {} reserved \\
                             domain-gap range(s) describe that closed stream and were not \\
                             published",
                            final_gap_reservation.gaps().len()""",
        """                            "a campaign rollover committed inside a compact write closed the \\
                             logical stream before terminal H4DG publication; reserved \\
                             domain-gap ranges describe that closed stream and were not \\
                             published{}",
                            \"\"""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T20 a rollover committed by a capture-loop scheduler write is forgotten by the "
        "terminal H4DG guard",
        ATTACH,
        """                } else if scheduler_stream_closed_by_write
                    || transport.current_logical_stream_id() != terminal_logical_stream
                {""",
        """                } else if transport.current_logical_stream_id() != terminal_logical_stream {""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T21 the scheduler H4DG write closes the stream without recording that it did",
        ATTACH,
        """                                        scheduler_stream_closed_by_write = true;
                                        append_terminal_error(""",
        """                                        append_terminal_error(""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T23 a rollover committed inside the frame-refusal publication is not recorded",
        ATTACH,
        """                                        scheduler_stream_closed_by_write = true;
                                        terminal_control_error = Some(""",
        """                                        terminal_control_error = Some(""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T24 the frame-refusal publication reports an exhausted uncertainty as a rollover",
        ATTACH,
        """    Err(format!(
        "frame refusal record for hour {refused_hour} could not be published: the campaign \\
         identity stayed uncertain across {FRAME_REFUSAL_UNCERTAIN_BOUNDARY_ATTEMPTS} attempts"
    ))""",
        """    Ok(FrameRefusalPublication::NotPublishedAtIdentityBoundary)""",
        "the_terminal_gap_publication_names_every_range_it_could_not_publish",
    ),
    (
        "T22 a failed D75 terminal publication is swallowed instead of aborting custody",
        ATTACH,
        """                                        Err(error) => {
                                            let abort = abort_terminal_d75_owner(
                                                &mut terminal_d75_d64_custody,
                                                &mut production_hook_registry,
                                            );
                                            return Err(match abort {
                                                Ok(()) => error,
                                                Err(abort) => {
                                                    format!(
                                                        "{error}; abort D75 after terminal sink failure: {abort}"
                                                    )
                                                }
                                            });
                                        }""",
        """                                        Err(error) => {
                                            let _ = error;
                                            TerminalFrameAttempt {
                                                outcome: CompactFrameWrite::RetryUncertainBoundary,
                                                attempts: 1,
                                            }
                                        }""",
        "the_d75_terminal_writer_refuses_an_uncertain_boundary_instead_of_claiming_a_write",
    ),
    (
        "T19 the D75 terminal writer treats an uncertain boundary as an accepted write",
        ATTACH,
        """            CompactFrameWrite::RetryUncertainBoundary => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "terminal H4CR met an uncertain campaign identity; nothing was written",
                ));
            }""",
        # MUTANT REPAIRED by lane/write_receipt: `accepted_outcome` holds the
        # sealed write, so claiming an acceptance that never happened has to
        # seal one. Still available, still caught by the same named test.
        """            CompactFrameWrite::RetryUncertainBoundary => {
                self.accepted_outcome = Some(
                    seal_completed_write(
                        SinkWriteOutcome {
                            file_requested: true,
                            file_complete: true,
                            pipe_requested: false,
                            pipe_complete: false,
                        },
                        bytes,
                    )
                    .expect("sealed"),
                );
                return Ok(bytes.len());
            }""",
        "the_d75_terminal_writer_refuses_an_uncertain_boundary_instead_of_claiming_a_write",
    ),
]


def run(cmd, **kw):
    return subprocess.run(
        cmd, cwd=NATIVE, env=ENV, capture_output=True, text=True, shell=True, **kw
    )


def revert(path):
    subprocess.run(["git", "checkout", "--", path], cwd=ROOT, shell=False, check=True)


def main():
    only = sys.argv[1:] if len(sys.argv) > 1 else None
    results = []
    for label, path, old, new, red_test in MUTATIONS:
        tag = label.split()[0]
        if only and tag not in only:
            continue
        src = open(path, encoding="utf-8").read()
        if src.count(old) != 1:
            results.append((label, "ANCHOR-MISS count=%d" % src.count(old)))
            print("[%s] ANCHOR MISS (%d)" % (tag, src.count(old)), flush=True)
            continue
        open(path, "w", encoding="utf-8").write(src.replace(old, new))
        try:
            if red_test is None:
                r = run("cargo check -p collector --all-targets")
                ok = r.returncode != 0
                detail = "COMPILE ERROR" if ok else "still compiles"
                results.append(
                    (label, ("KILLED (%s)" % detail) if ok else "SURVIVOR (%s)" % detail)
                )
            else:
                full = "%s::%s" % (TESTMOD, red_test)
                r = run('cargo nextest run -p collector -E "test(=%s)"' % full)
                out = r.stdout + r.stderr
                if "1 test run" not in out and "1 tests run" not in out:
                    if "error: could not compile" in out or "error[E" in out:
                        results.append(
                            (label, "COMPILE ERROR (expected %s to go red)" % red_test)
                        )
                    else:
                        results.append((label, "TEST-NOT-SELECTED for %s" % full))
                else:
                    killed = r.returncode != 0
                    results.append(
                        (
                            label,
                            ("KILLED by %s" % red_test)
                            if killed
                            else "SURVIVOR (%s stayed green)" % red_test,
                        )
                    )
            print("[%s] %s" % (tag, results[-1][1]), flush=True)
        finally:
            revert(path)
    print("\n==== MUTATION RESULTS ====")
    for label, verdict in results:
        print("%-90s %s" % (label, verdict))


if __name__ == "__main__":
    main()
