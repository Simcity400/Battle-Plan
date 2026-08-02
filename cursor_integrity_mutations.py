"""Mutation battery for lane/cursor_integrity.

Each entry mutates PRODUCTION code and names the test that must go red.
A mutation whose named test stays green is a SURVIVOR and is reported.

Run from the worktree root. Reverts the touched file after every mutation.
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
TESTMOD = "attach_log::frame_refusal_sequence_tests"

ENV = dict(os.environ, CARGO_TARGET_DIR=os.path.join(ROOT, ".cargo-target"))

# (label, file, old, new, expected_red_test_or_None_for_compile_error)
MUTATIONS = [
    (
        "M1 refusal call site takes the STAGED cursor again (the shipped defect)",
        ATTACH,
        """                                match publish_frame_refusal_record(
                                    &mut transport,
                                    &mut session_sequence,""",
        """                                match publish_frame_refusal_record(
                                    &mut transport,
                                    &mut next_sequence,""",
        None,  # must not compile
    ),
    (
        "M2 resync always restarts numbering",
        ATTACH,
        """            if self.logical_stream == current {
                return false;
            }""",
        """            if false {
                return false;
            }""",
        "a_reset_on_the_same_logical_stream_does_not_rewind_the_published_cursor",
    ),
    (
        "M3 resync never restarts numbering",
        ATTACH,
        """            self.logical_stream = current;
            self.next = 1;
            true""",
        """            self.logical_stream = current;
            false""",
        "a_reset_onto_a_new_logical_stream_restarts_numbering_at_one",
    ),
    (
        "M4 commit_staged drops the rewind check",
        ATTACH,
        """            if staged_after_frame < self.next {""",
        """            if false {""",
        "committing_a_staged_cursor_behind_the_published_one_is_refused",
    ),
    (
        "M5 write_compact_frame calls an uncertain boundary a rollover",
        ATTACH,
        """            CampaignIdentityBoundary::RetryUncertainBoundary => {
                Ok(CompactFrameWrite::RetryUncertainBoundary)
            }""",
        """            CampaignIdentityBoundary::RetryUncertainBoundary => {
                Ok(CompactFrameWrite::RolledOver)
            }""",
        "an_uncertain_boundary_writes_nothing_and_is_not_a_rollover",
    ),
    (
        "M6 refusal treats an uncertain boundary as terminal (the old collapse)",
        ATTACH,
        """            CompactFrameWrite::RetryUncertainBoundary => {
                thread::sleep(CAMPAIGN_IDENTITY_UNCERTAIN_BOUNDARY_BACKOFF);
            }""",
        """            CompactFrameWrite::RetryUncertainBoundary => {
                return Ok(FrameRefusalPublication::NotPublishedAtIdentityBoundary)
            }""",
        "a_refusal_re_attempts_across_an_uncertain_boundary_and_still_spends_one_sequence",
    ),
    (
        # ANCHOR REPAIRED by lane/write_receipt. The refusal no longer advances
        # the cursor on the caller's say-so: it takes the receipt out of the
        # write that proved the H4FR record reached every sink and spends it
        # through the single-record door. Same mutation, same property, same
        # named test -- the line it deletes moved.
        "M7 a published refusal does not advance the cursor",
        ATTACH,
        """                session_sequence.commit_one_published_record(receipt)?;
                return Ok(FrameRefusalPublication::Published);""",
        """                let _ = receipt;
                return Ok(FrameRefusalPublication::Published);""",
        "refused_hours_take_consecutive_published_sequences_and_the_stream_certifies",
    ),
    (
        "M8 checked_successor tolerates overflow",
        ATTACH,
        """            self.next
                .checked_add(1)
                .ok_or_else(|| "compact published sequence overflow".to_string())""",
        """            Ok(self.next.wrapping_add(1))""",
        "a_published_cursor_at_the_maximum_refuses_to_advance",
    ),
    (
        "M9 advance_one_saturating does nothing (legacy JSONL line numbering)",
        ATTACH,
        """            self.next = self.next.saturating_add(1);""",
        """            self.next = self.next;""",
        "an_accepted_record_advances_the_published_cursor_by_exactly_one",
    ),
    (
        "M10 the refusal reserves no successor before writing",
        ATTACH,
        """    session_sequence.checked_successor().map_err(|_| {
        "frame refusal record cannot advance the compact sequence without overflow".to_string()
    })?;""",
        """""",
        "a_refusal_that_could_not_be_numbered_next_writes_no_bytes_at_all",
    ),
    (
        "M11 current_logical_stream_id always reports a legacy stream",
        ATTACH,
        """            AuthoritativeSinkSet::IdentityV1 { barrier, .. } => Some(barrier.logical_stream_id),""",
        """            AuthoritativeSinkSet::IdentityV1 { .. } => None,""",
        # Originally specified as "must not compile", which was wrong: returning
        # None for IdentityV1 is a perfectly well-typed expression, so that check
        # could only ever report a survivor. It needs a TEST -- and needed one to
        # exist, because nothing covered the IdentityV1 side of this accessor.
        "an_identity_v1_transport_reports_the_logical_stream_that_restarts_numbering",
    ),
    (
        "M12 current_logical_stream_id invents an id for a legacy stream",
        ATTACH,
        """            AuthoritativeSinkSet::Legacy { .. } => None,
        }
    }

    fn current_campaign_root""",
        """            AuthoritativeSinkSet::Legacy { .. } => Some(0),
        }
    }

    fn current_campaign_root""",
        "a_legacy_transport_reports_no_logical_stream_id",
    ),
    (
        # Review finding: making the direct argument a type error is not enough
        # if a SECOND published cursor can be conjured. While the type was Copy,
        # the laundering below compiled, and it is the shipped defect wearing
        # the new type: the record is stamped with the staged number while the
        # live cursor -- the one every assertion watches -- never moves.
        #
        # REPLACED by lane/write_receipt, on its own review finding. The
        # laundering mutation this entry used to carry --
        #
        #     let mut forged = session_sequence;
        #     forged.commit_staged(next_sequence).ok();
        #
        # -- still fails to compile, but no longer for the reason it names.
        # `commit_staged` takes a write receipt now, so the second line is a
        # type error on its ARGUMENT and stays one with `Copy` restored. The
        # entry reported KILLED while proving nothing: a proof that went stale
        # without going red, which is the one failure mode this battery must not
        # have.
        #
        # The property is now mutated directly -- put `Copy, Clone` back on the
        # cursor -- and read off the declaration by a named test, the same shape
        # `write_receipt_mutations.py` W9 uses for the receipt. A copy behaves
        # exactly like the original until it is spent twice, so the derive list
        # is the only place the difference is visible.
        "M13 the published cursor becomes duplicable again",
        ATTACH,
        """    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct PublishedSequence {""",
        """    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub(super) struct PublishedSequence {""",
        "the_published_cursor_is_move_only",
    ),
]


def run(cmd, **kw):
    return subprocess.run(cmd, cwd=NATIVE, env=ENV, capture_output=True, text=True, shell=True, **kw)


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
                results.append((label, ("KILLED (%s)" % detail) if ok else "SURVIVOR (%s)" % detail))
            else:
                full = "%s::%s" % (TESTMOD, red_test)
                r = run("cargo nextest run -p collector -E \"test(=%s)\"" % full)
                out = r.stdout + r.stderr
                if "1 test run" not in out and "1 tests run" not in out:
                    results.append((label, "TEST-NOT-SELECTED for %s" % full))
                else:
                    killed = r.returncode != 0
                    results.append((label, ("KILLED by %s" % red_test) if killed else "SURVIVOR (%s stayed green)" % red_test))
            print("[%s] %s" % (tag, results[-1][1]), flush=True)
        finally:
            revert(path)
    print("\n==== MUTATION RESULTS ====")
    for label, verdict in results:
        print("%-70s %s" % (label, verdict))


if __name__ == "__main__":
    main()
