# lane/write_receipt

Base `322fb9af` (lane/terminal_write_path). All line numbers are at lane HEAD
unless marked `(base)`.

## What was open

`lane/cursor_integrity` made the published cursor a type a staged number cannot
inhabit, and stated its own residual on that type:

> The real remaining gap is narrower and is on `commit_staged`: it takes the
> caller's word that those bytes reached every sink.

That word is the whole of what stood between a not-written frame and a
committed cursor. `lane/terminal_write_path` is, in its entirety, a list of arms
that got the word wrong: an `if let ... = Written(outcome)` with no else, a
`_ => break` covering three different endings, two `RolledOver` arms invoking a
reset that could never run. Each was fixed by hand, and each fix is one edit
away from being undone.

## The receipt

`FrameWriteReceipt` (`:32669`) is opaque, move-only (no `Copy`, no `Clone`, no
`Default`, private fields) and has no constructor from numbers.

**One mint.** `seal_completed_write` is called from exactly one place,
`CaptureTransport::write` (`:91009`) -- the function that hands the bytes to the
sinks. It mints only when `SinkWriteOutcome::complete()`, i.e. when every
requested sink took the whole record batch.

**No caller's number reaches the cursor.** Both of the receipt's numbers are
read back out of the buffer the sinks accepted, by
`compact_record_sequence_bounds` -- the same walk the IdentityV1 progress
counter already uses (`:91348`). `published_before` is the first record's
sequence; `staged_after_frame` is one past the last's.

**The not-written arms are uninhabitable, not merely discouraged.**
`CompactFrameWrite::Written` now carries a `WrittenFrame` (outcome + receipt).
`RolledOver` and `RetryUncertainBoundary` are still unit variants: an arm that
meets one has no value to take a receipt out of, so a commit there does not
type-check. That is mutation W1 -- it fails with `cannot find value`, not with
an assertion.

**`complete()` is by reference.** Every existing arm shape
(`Written(outcome) if outcome.complete()`, `Written(outcome) if !outcome.complete()`)
compiles unchanged, because a match guard cannot move its binding. That is what
kept the terminal lane's structural assertions and its 24-mutation battery on
their existing anchors.

## The seven sites, and the writer that authorizes each

| Site | What it publishes | Writer |
| --- | --- | --- |
| `:3467` | scheduler DATA, capture loop | `write_compact_frame` |
| `:3529` | scheduler H4DG gaps, capture loop | `write_compact_frame` |
| `:3959` | scheduler-OFF compact write | `write_compact_frame` |
| `:4532` | terminal DATA | `TerminalD75D64Custody::publish` (windows, custody present) or `publish_terminal_compact_frame` |
| `:4769` | terminal H4DG gaps | `write_compact_frame` |
| `:32529` | pre-boundary scheduler DATA | `write_preboundary_compact_frame` |
| `:32579` | pre-boundary scheduler H4DG | `write_preboundary_compact_frame` |

Plus the frame-refusal record's own single-sequence commit (`:33267`), which is
not a frame commit and goes through its own door (below).

All nine go through `commit_published_frame` (`:32794`) or
`commit_one_published_record`, and nothing ahead of the receipt module names
`commit_staged` any more --
`write_receipt_tests::every_frame_commit_goes_through_the_receipt_door` asserts
that on the source.

Three sites needed reshaping rather than a signature swap.

### 1. The capture-loop scheduler DATA match now yields the write

`(base) :3381`-`:3461`. The commit sat below the match. Every other ending
leaves through a `break 'capture` or a `continue 'capture`, so the only value
that can fall out of the match is a frame all the sinks took -- which is exactly
what the commit asserts. The match is now an expression and the arm returns the
`WrittenFrame`.

### 2. The scheduler-OFF site committed unconditionally -- FIXED

`(base) :3970`. The commit sat *below* the `if !records.is_empty()` that guarded
the write, so an empty build ran it too. It was harmless only because an empty
build stages nothing, and nothing checked that. A receipt cannot be minted for a
write that never happened, so the branch is now explicit: an empty frame commits
nothing, and an empty build whose staged cursor MOVED fails the run rather than
passing silently. That is a real (if narrow) hole closed, not a refactor: the
old line would have committed whatever `build_compact_extraction_gate_records`
returned on a path that wrote no bytes at all.

### 3. The D75 custody writer carries the sealed write out

`D75TerminalAuthoritativeWriter::accepted_outcome` (`:33818`) held a bare
`SinkWriteOutcome`; it now holds the `WrittenFrame`. The terminal DATA commit on
the Windows custody path is therefore authorized by the buffer the a729 runtime
actually published through `Write::write`, not by the caller's `&records`. This
is strictly stronger than minting at the call site would have been.

## The two refusals on `commit_staged`, and why both are needed

* **Stale receipt.** `staged_after_frame < self.next`: a proof for a frame the
  stream has already moved past. This is the predecessor lane's rewind check,
  unchanged, and cursor-battery M4 still kills its removal.
* **Wrong frame.** `published_before != self.next`: a receipt for a frame
  numbered from a different cursor. It can be *ahead* of the live cursor, which
  is exactly where the rewind check cannot see it. Two real writes, two live
  receipts, spent in the wrong order, is the case
  `a_receipt_from_a_different_frame_is_refused_by_the_cursor_it_is_offered_to`
  drives; mutation W4 kills its removal.

`commit_published_frame` adds a third, at the door rather than on the cursor:
the receipt's `staged_after_frame` must equal the number the frame BUILD reached.
The receipt is the authority for where the cursor lands -- it read that off the
wire -- and the build's number is the cross-check. They disagree only if the
builder numbered records it did not emit or emitted records it did not number.

`commit_one_published_record` is the single-record door the frame-refusal
publication uses. It pins "a refused hour spends exactly one sequence" -- the
property the shipped 75 -> 222 stamp broke -- in the type instead of at the call
site.

## The honest limit

`seal_completed_write` is reachable from anywhere in `attach_log.rs`. Within one
module tree there is no way to make a `pub(super)` mint unreachable from the
parent, so the receipt is unforgeable in the sense that **no value you hold
yields one**, not in the sense that no code could call the mint. A deliberate
second call could mint for bytes no sink was ever offered.

That is not a wrong argument in a call; it is a new call. It is pinned as a
property of the file --
`write_receipt_tests::exactly_one_production_call_site_mints_a_write_receipt`
asserts there are exactly two occurrences of `seal_completed_write(` in the
production source (the definition and one call) and that the call is inside the
transport write. Mutation W8 adds a second mint in
`write_preboundary_compact_frame`; it compiles, it is well-typed, and the test
catches it.

This is stated the way the predecessor lane stated its `Copy` limit, and for the
same reason: a doc comment claiming more than the type enforces is how the
`Copy` hole survived review in the first place.

## What did NOT change

* `RetryUncertainBoundary` still means nothing happened. No receipt, no commit,
  cursor untouched, re-attempt with nothing reset.
* The terminal re-attempt budget (`TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS = 20`
  on the 50 ms back-off) and its exhaustion behaviour are byte-identical.
* Both successor-stream latch conditions (`scheduler_stream_closed_by_write` and
  the `current_logical_stream_id()` comparison) are untouched.
* The cursor is still not `Copy` and still has no constructor from a number.
* The legacy JSONL line paths still write through `transport.write(...)?
  .complete()` and still move the cursor with `advance_one_saturating`. A line
  is not a numbered compact record, so it mints no receipt -- and must not.

## The D75 seam, restated (out of scope)

The receipt does not want a seam in `lanes/fired_event_popups_live`. The
terminal DATA path's Windows custody branch already routes its outcome back
through `D75TerminalAuthoritativeWriter`, and carrying the `WrittenFrame` along
that same channel needed no API change in the a729 crate.

The residual recorded by the previous lane at
`lanes/fired_event_popups_live/src/a729_runtime.rs:1111` is unchanged and
untouched: `publish_terminal_h4cr_to_authoritative_sink` classifies EVERY error
out of the collector's `Write` as `retry_permit: None,
sink_state_indeterminate: true`, so a `RetryUncertainBoundary` that provably
wrote nothing cannot be re-attempted on the same budget every other terminal
publication uses. It is fail-closed (the caller aborts D75 custody and fails the
run) and it is another crate's ruling.

## Tests

`extractor/native/collector/src/attach_log/write_receipt_tests.rs`, module
`attach_log::write_receipt_tests`, 12 tests, plus
`frame_refusal_sequence_tests::the_published_cursor_is_move_only`, driven against a real legacy
file transport so the receipts under test come out of writes that really
happened.

Batteries, all run at this HEAD from a committed tree
(`extractor/native/runs/write_receipt/`):

| Battery | Result |
| --- | --- |
| `write_receipt_mutations.py` | 10/10 killed, 0 anchor misses |
| `cursor_integrity_mutations.py` | 13/13 killed, 0 anchor misses |
| `terminal_write_path_mutations.py` | 24/24 killed, 0 anchor misses |

Both prior harnesses had `ROOT` hardcoded to their own worktree, which made them
mutate -- and `git checkout --` revert -- files in a tree the runner was not
looking at. `ROOT` is now derived from the harness file's own location.

Four entries moved with production, and each is annotated in place:

* cursor **M7**: the refusal spends its sequence through the receipt door, so
  the line the mutation deletes is a different line. Same property, same named
  test.
* terminal **T7** and **T19**: both mutants fabricated a bare `SinkWriteOutcome`
  into a place that now holds a sealed write, so both fabrications go through
  the mint. Worth keeping exactly as they were: the receipt does not stop a
  shutdown from LYING about its verdict, only from moving the cursor on one.
* cursor **M13** is REPLACED, on this lane's own Codex review. Its laundering
  mutation (`let mut forged = session_sequence; forged.commit_staged(next_sequence)`)
  still fails to compile, but no longer for the reason it names: `commit_staged`
  takes a receipt now, so the second line is a type error on its ARGUMENT and
  would stay one with `Copy` restored. It reported KILLED while proving nothing
  -- a proof that went stale without going red, the one failure mode a battery
  must not have. The property is now mutated directly (put `Copy, Clone` back on
  the cursor) and read off the declaration by
  `frame_refusal_sequence_tests::the_published_cursor_is_move_only`, the same
  shape W9 uses for the receipt.
* terminal **T15** is upgraded, not repaired. "A not-written terminal DATA frame
  commits the published cursor anyway" used to compile and be caught by a
  structural assertion; it no longer type-checks, so its expectation becomes the
  compile-proof.

Suite, serial, `--no-fail-fast`:

| | run | passed | failed | skipped |
| --- | --- | --- | --- | --- |
| base `322fb9af` | 6453 | 6445 | 8 | 9 |
| lane HEAD | 6466 | 6458 | 8 | 9 |

The 8 failures are the same pre-existing set, listed in
`runs/write_receipt/head_failures.txt` and `suite_head_final.log`; none is in this lane's regions.
`attach_log::tests::d55_and_d67_hook_events_remain_reserved_until_sink_success`
was checked against the base directly -- every count it asserts is identical at
both revisions.
