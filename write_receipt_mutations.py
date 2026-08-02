"""Mutation battery for lane/write_receipt.

Each entry mutates PRODUCTION code and names the test that must go red, or
`None` for entries whose whole point is that the mutation no longer type-checks.
A mutation whose named test stays green is a SURVIVOR and is reported.

Run from the worktree root. Reverts the touched file after every mutation, so
COMMIT before running.
"""

import subprocess
import sys
import os

# The worktree this battery is run from, derived from the file's own location.
ROOT = os.path.dirname(os.path.abspath(__file__))
NATIVE = os.path.join(ROOT, "extractor", "native")
ATTACH = os.path.join(NATIVE, "collector", "src", "attach_log.rs")
TESTMOD = "attach_log::write_receipt_tests"

ENV = dict(
    os.environ,
    CARGO_TARGET_DIR=os.path.join(ROOT, ".cargo-target"),
    CARGO_INCREMENTAL="0",
)

# (label, file, old, new, expected_red_test_or_None_for_compile_error)
MUTATIONS = [
    (
        # The lane's thesis, stated as a compile error. `RolledOver` carries no
        # value, so the arm that meets it has nothing to hand to the commit
        # door. Not "must not commit" -- CANNOT NAME a commit.
        "W1 a committed rollover commits the frame it did not write",
        ATTACH,
        """                                CompactFrameWrite::RolledOver => {
                                    // The frame belonged to the closed stream.
                                    // Its staged trackers drop uncommitted, so
                                    // every baseline rolls back with it.
                                    reset_campaign_scoped_capture_state!('capture);
                                    continue 'capture;
                                }""",
        """                                CompactFrameWrite::RolledOver => {
                                    commit_published_frame(
                                        &mut session_sequence,
                                        outcome,
                                        next_sequence,
                                    )?;
                                    reset_campaign_scoped_capture_state!('capture);
                                    continue 'capture;
                                }""",
        None,  # must not compile: there is no `outcome` in a not-written arm
    ),
    (
        # The residual the predecessor lane named, restored verbatim: the
        # terminal DATA site trusting its caller again.
        "W2 the terminal DATA commit goes back to taking the caller's word",
        ATTACH,
        """                                CompactFrameWrite::Written(outcome) if outcome.complete() => {
                                    commit_published_frame(
                                        &mut session_sequence,
                                        outcome,
                                        next_sequence,
                                    )?;""",
        """                                CompactFrameWrite::Written(outcome) if outcome.complete() => {
                                    session_sequence.commit_staged(next_sequence)?;""",
        None,  # must not compile: a u64 is not a receipt
    ),
    (
        "W3 the mint ignores whether every sink took the bytes",
        ATTACH,
        """        let receipt = match (outcome.complete(), compact_record_sequence_bounds(bytes)) {""",
        """        let receipt = match (true, compact_record_sequence_bounds(bytes)) {""",
        "a_write_that_did_not_reach_every_sink_mints_no_receipt",
    ),
    (
        "W4 the cursor stops checking which frame a receipt belongs to",
        ATTACH,
        """            if published_before != self.next {
                return Err(format!(
                    "a write receipt proves a frame numbered from published cursor \\
                     {published_before}, but this stream's cursor stands at {}; the receipt \\
                     belongs to a different frame",
                    self.next
                ));
            }""",
        """            if false {
                return Err(format!(
                    "a write receipt proves a frame numbered from published cursor \\
                     {published_before}, but this stream's cursor stands at {}; the receipt \\
                     belongs to a different frame",
                    self.next
                ));
            }""",
        "a_receipt_from_a_different_frame_is_refused_by_the_cursor_it_is_offered_to",
    ),
    (
        "W5 the commit door stops cross-checking the build against the wire",
        ATTACH,
        """    if receipt.staged_after_frame() != staged_after_frame {""",
        """    if false {""",
        "a_commit_whose_receipt_disagrees_with_the_build_is_refused",
    ),
    (
        "W6 a refused hour may spend a whole frame's worth of sequences",
        ATTACH,
        """            if staged_after_frame != self.checked_successor()? {""",
        """            if false {""",
        "the_single_record_door_refuses_a_receipt_for_a_whole_frame",
    ),
    (
        # The site that used to commit unconditionally below the write guard.
        "W7 an empty compact frame's staged cursor is trusted again",
        ATTACH,
        """                            if next_sequence != session_sequence.get() {
                                return Err(format!(
                                    "an empty compact frame staged the sequence up to \\
                                     {next_sequence} from a published cursor at {} without \\
                                     writing anything",
                                    session_sequence.get()
                                ));
                            }""",
        """                            let _ = next_sequence;""",
        "an_empty_compact_frame_commits_nothing_and_says_why",
    ),
    (
        # A SECOND mint. It compiles, it is well-typed, and it hands out a
        # receipt for a claimed-complete write regardless of what the sinks did
        # -- which is exactly why "there is one mint" has to be asserted on the
        # source rather than inferred from the type.
        "W8 a second mint appears outside the function that writes",
        ATTACH,
        """    fn write_preboundary_compact_frame(&mut self, bytes: &[u8]) -> Result<WrittenFrame, String> {
        self.write(bytes)
    }""",
        """    fn write_preboundary_compact_frame(&mut self, bytes: &[u8]) -> Result<WrittenFrame, String> {
        self.write(bytes)?;
        seal_completed_write(
            SinkWriteOutcome {
                file_requested: true,
                file_complete: true,
                pipe_requested: false,
                pipe_complete: false,
            },
            bytes,
        )
    }""",
        "exactly_one_production_call_site_mints_a_write_receipt",
    ),
    (
        # The predecessor lane's M13 in this lane's terms: a proof that can be
        # copied is a proof that can be spent twice.
        "W9 the receipt becomes copyable",
        ATTACH,
        """    #[derive(Debug)]
    pub(super) struct FrameWriteReceipt {""",
        """    #[derive(Debug, Clone, Copy)]
    pub(super) struct FrameWriteReceipt {""",
        "the_receipt_is_move_only",
    ),
    (
        # The receipt's numbers must come off the wire. This one takes the FIRST
        # record's sequence for both ends -- a receipt that always says "one
        # record" -- which is well-typed and wrong.
        "W10 the receipt stops reading the frame's end off the wire",
        ATTACH,
        """                staged_after_frame: last.checked_add(1).ok_or(""",
        """                staged_after_frame: first.checked_add(1).ok_or(""",
        "a_receipt_numbers_the_frame_from_the_bytes_the_sinks_accepted",
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
        print("%-70s %s" % (label, verdict))


if __name__ == "__main__":
    main()
