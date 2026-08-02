# lane/terminal_write_path

Base `16640b81` (lane/cursor_integrity). All line numbers are at lane HEAD unless
marked `(base)`.

## What the terminal path is

The compact production capture worker is one function, `write_attach_heartbeat`
(`extractor/native/collector/src/attach_log.rs:1344`-`5566`). Its capture loop is
`'capture: while Instant::now() <= deadline` at `:2889`; everything after `:4061`
is the shutdown sequence. There is no separate teardown function, so "terminal"
means "reached after the loop", and every `break 'capture` in the loop lands
there.

The shutdown sequence, in order:

| Step | Where | What it does |
| --- | --- | --- |
| 1 | `:4061` | capture-ready preflight and A30 evidence completeness |
| 2 | `:4081` | `read_only_scheduler_runtime.take()`, `shutdown_and_flush_latest_pending()` — joins the observer thread, closes only the *unissued* frontier; the in-flight DATA request is retained |
| 3 | `:4085` | final telemetry, then an unconditional `record_attach_worker_error("scheduler_telemetry", …)` diagnostic |
| 4 | `:4192`-`:4520` | **terminal DATA publication** of `pending_scheduler_frame`: D45/D55/D57/D67 merges, D86/D90, D75/D64 custody, A30 payloads, the final scheduler probe, then the write |
| 5 | `:4521`-`:4600` | **terminal H4DG publication**: `flush_remaining_and_reserve_control_gaps()` closes the still-in-flight request and returns every exact loss range; each leading group is built and written |
| 6 | `:4608` | D11 sink-failure report over H4CT (`terminal_sink_error`) |
| 7 | `:4630`+ | A40/A39 evidence, hook teardown, `aggregate_terminal_errors` → `Err` |

Three shutdown shapes reach it: an orderly detach (`transport.stop_requested()`
or the duration deadline), a mid-write stop (any `break 'capture` from a sink or
control failure), and a rollover observed *during* shutdown — the terminal writes
go through `write_compact_frame`, which inspects the campaign identity on every
call, so both not-written verdicts are live at teardown.

`terminal_control_error` is the fail-closed channel: `aggregate_terminal_errors`
at `:5541` turns it into the worker's `Err`.

## The four findings

### 1. A typed gap record was dropped on `_ => break` — FIXED

`(base) :4491`-`:4503`. The terminal H4DG loop matched
`Written(outcome) if outcome.complete()` and sent **everything else** to
`_ => { break; }`: a partial sink write, a committed rollover, and an identity
that would not settle. `final_gap_reservation` then dropped uncommitted, whose
`Drop` (`:31423`) calls `rollback_reserved_control()` — "rolled back to the
scheduler", except `final_scheduler_runtime` is dropped a few statements later
and nothing runs after it. `terminal_control_error` was untouched, so every
reserved `DomainGapRange` disappeared and the run reported success.

Now exhaustive (`:4561`-`:4593` region). Each non-published ending appends a
typed `terminal_control_error` naming the ending *and* the number of reserved
ranges it lost. A fourth accounting was added for the `delivery().terminal`
branch at `:4530 (base)`, which discarded the same reservation silently when the
sink had already failed.

### 2. The terminal DATA write ignored both not-written variants — FIXED

`(base) :4342`. `if let CompactFrameWrite::Written(outcome) = terminal_write_outcome`
with **no else**. `RolledOver`, `RetryUncertainBoundary`, and
`Written(!complete)` all fell out of the bottom: nothing committed, nothing
rolled back that mattered, nothing said. The only thing that ever noticed was the
Windows D75 custody abort at `(base) :4433`, which is conditional on a registry
that holds the XUI21 producer edge *and* D64 — so a capture without that custody,
or any non-Windows build, ended reporting success while its last assembled hour
had never reached a sink.

Now a `match` with a `not_published` arm that appends a typed error naming which
of the three endings it met and how many attempts it spent. Appended via
`append_terminal_error`, never assigned: an earlier failure is usually *why* the
write did not land, and the existing comment at `(base) :4160` documents exactly
this clobbering destroying the answer in runs 04 and 05.

### 3. A terminal retry discarded rather than re-attempted — FIXED

`RetryUncertainBoundary` is the variant that means nothing happened: not the
identity, not the logical stream, not the published cursor. Both terminal writes
re-attempted it zero times. The old comment at `(base) :4569` stated the reason —
"the loop deadline is already spent, so an uncertain identity gets no back-off
here" — which is a reason not to retry *forever*, not a reason not to retry.

`publish_terminal_compact_frame` (`:32748`) re-attempts on the capture loop's own
`CAMPAIGN_IDENTITY_UNCERTAIN_BOUNDARY_BACKOFF` (50 ms), bounded by
`TERMINAL_UNCERTAIN_BOUNDARY_ATTEMPTS = 20` — about one second, the same budget
`publish_frame_refusal_record` already uses for the same verdict. Only
`RetryUncertainBoundary` is re-attempted: a `RolledOver` boundary was already
committed, and an incomplete write already reached the sinks that accepted it.
Exhaustion returns the verdict it actually got, carrying the attempt count; it
never fabricates a success.

The H4DG loop uses **one** budget for the whole publication rather than one per
group, so a stream holding many reserved groups cannot multiply the teardown
wait. The DATA and H4DG budgets are spent one after the other, so the worst case added
teardown time is the **sum**: ~1.9 s.

### 4. Scheduler rollover arms abort on the macro guard — FAIL-CLOSED, alternative documented

`reset_campaign_scoped_capture_state!` (`:2725`) opens with

```rust
if pending_scheduler_frame.is_some() || read_only_scheduler_runtime.is_some() {
    terminal_control_error = Some(
        "campaign reset reached mutable state before old scheduler DATA-or-H4DG custody was finalized");
    break $capture_loop;
}
```

The guard is right, and it fires unconditionally at two of the macro's five call
sites. `(base) :3347` is reached only from the pending-scheduler-frame branch and
`(base) :3443` only from inside `if let Some(runtime) = read_only_scheduler_runtime.as_ref()`,
so the runtime is live by construction on both. Those arms read as "reset and
continue"; they were an abort wearing recovery's comment.

The guard cannot be relaxed for them, because by the time either arm runs the
boundary is already **committed**: `write_compact_frame` (`:90362`) calls
`self.rollover_campaign(...)` itself before returning `RolledOver`. The old
logical stream is closed while the old scheduler still holds this frame's
in-flight request and every range reserved against it, and those can be published
to neither the closed stream nor the successor. The loop's own boundary handling
avoids this by *ordering* the steps — inspect, then
`finalize_scheduler_before_identity_boundary` (`:32199`), then `rollover_campaign`
(`:2955`/`:3026`/`:3012`) — and a write that performs its own rollover cannot be
given that ordering from outside.

**Implemented (fail-closed):** both arms now state the actual cause and
`break 'capture`. Behaviour is unchanged — the guard broke with an error too —
but the message names the committed rollover and the custody it stranded instead
of a generic "reached mutable state", and no dead recovery path remains.

**Documented alternative, not implemented:** give the terminal and scheduler
writes a variant of `write_compact_frame` that *returns* the observed
`CampaignIdentityBoundary::Rollover` instead of committing it. The caller could
then restore the frame into `pending_scheduler_frame`, run
`finalize_scheduler_before_identity_boundary` (which publishes old-identity bytes
through `write_preboundary_compact_frame` to the still-open old stream), and only
then perform the rollover. At shutdown it could go further and never perform it:
the capture is ending, so an observed-but-unserviced boundary costs nothing and
the held records reach the stream they belong to. That is strictly more data and
no false claim, but it changes what an observed boundary guarantees on the H4CT
identity protocol (CRP1/CRA1/CID1 acknowledgement), which is a ruling above a
write site. The trade-off is recorded in the code at both rollover arms.

The third macro call site, `(base) :3847`, is the scheduler-*off* compact write.
No runtime exists there, the guard passes, and its reset is genuinely reachable —
left untouched, and a test pins that it still resets.

## Opportunistic fix: the sweep thread was not always joined

`heap_sweep_shutdown()` (`:103175`) stops and joins the background heap-sweep
thread. It was called only at `write_attach_heartbeat`'s three orderly returns
(`:5545`, `:5552`, `:5564`). That function has hundreds of `?` sites and several
mid-teardown `return Err`s (`:4283`, `:4325`, `:4352`, `:4370`, `:4400`,
`:4438`), and an error taken through any of them skipped the join. `attach_worker`
then reaches `FreeLibraryAndExitThread` at `:1177` whenever
`module_may_be_unloaded()` is true — which is exactly the passive capture that
installed no detour, and exactly the capture most likely to end on an early
error. A live thread would then be executing an unmapped image while the game
process runs on.

`attach_worker` now calls `heap_sweep_shutdown()` unconditionally right after the
capture returns (`:1106`). It is idempotent: the service is taken out of its cell
by the first call.

## Tests

`extractor/native/collector/src/attach_log/terminal_write_path_tests.rs`, module
`attach_log::terminal_write_path_tests`, 11 tests.

An anchor miss is the one failure mode this harness must not have: it reports as
neither killed nor survived, and silently drops the mutation from the battery.
Four anchors drifted when the successor-stream flag landed and were repaired and
re-verified; the run log is the whole battery in one pass, not an accumulation.

Four drive `publish_terminal_compact_frame` over a scripted verdict sequence.
That is the only way to reach it: `RetryUncertainBoundary` is produced solely by
an `IdentityV1` transport whose live campaign-identity read disagreed with itself
across two reads, and every test-constructible transport (`CaptureTransport::legacy`)
answers `Stable` unconditionally. The script's tail returns `Err` once the budget
is exceeded, so an unbounded re-attempt fails its test instead of hanging the
suite.

The rest are source-structure assertions, the established pattern for this worker
(`attach_log::tests::campaign_boundary_finalizes_scheduler_before_closing_old_identity`
does the same). Comments are stripped before matching, because several of these
arms quote the construct they replaced.

## Also fixed after review: old-stream gaps into the successor stream

Found by the Codex review, and the worst of the set. After a rollover committed
inside the terminal DATA write, shutdown went straight on to
`flush_remaining_and_reserve_control_gaps()`, which closes the still-in-flight
scheduler request into H4DG ranges — and then wrote them. To the **successor**
stream, numbered from a published cursor that still counts the closed stream's
records, into a stream that starts at sequence one. That is the previous lane's
cursor defect running the other way: records stamped for one stream landing in
another.

The second review round found the same defect through an earlier door: a latch on
STATE cannot see an EVENT that happened before it. The capture loop's own
scheduler writes commit a rollover and then `break 'capture` into the terminal
sequence, by which point the successor already IS the current stream and any
latch taken there compares equal.

Both are closed, by two conditions that cover two windows and where neither
covers the other:

* `scheduler_stream_closed_by_write` — a flag declared outside the capture loop
  and set at all four sites that reach teardown after a write committed a
  boundary the caller never got to order around its custody: the scheduler DATA
  write, the scheduler H4DG write, the frame-refusal publication, and the
  terminal DATA write. It records the event, so it survives the break.

  The frame-refusal site is the one the third review round found. Its
  `FrameRefusalPublication::NotPublishedAtIdentityBoundary` verdict looked like
  it might mean two things — its doc said "rollover or an uncertain boundary
  retry" — and the flag is only safe to set on it while it means exactly one.
  It does: `publish_frame_refusal_record` returns it solely from its
  `CompactFrameWrite::RolledOver` arm and reports an identity that never settled
  as an `Err`. The doc now says so and a test pins both halves, because the day
  that stops being true the flag would suppress H4DG on a stream still open.
* `transport.current_logical_stream_id() != terminal_logical_stream` — latched
  before the terminal DATA publication. It catches a rollover committed after
  that latch, and keeps the guard working if a future write site forgets the
  flag.

The loop's own boundary handling sets neither, and must not: it observes the
boundary first, finalizes the old scheduler's custody to the still-open old
stream, performs the rollover and resyncs the cursor. Nothing is stranded there.
A legacy transport reports `None` forever and never rolls, so the comparison is
equal on every capture that cannot reach the situation.

## Known residual: the D75 terminal writer does not re-attempt

`D75TerminalAuthoritativeWriter` (`:33473`) is the one terminal publication that
meets `RetryUncertainBoundary` and does not re-attempt it. Not for a reason this
crate can fix: `publish_terminal_h4cr_to_authoritative_sink`
(`extractor/native/lanes/fired_event_popups_live/src/a729_runtime.rs:1111`)
classifies **every** error out of that `Write` as
`retry_permit: None, sink_state_indeterminate: true`, so the single retry permit
that API already carries (`attach_log.rs:33226`) is unreachable for a refusal
that provably wrote no bytes.

It is fail-closed, not silent: the caller aborts the D75 custody and fails the
run, and the custody receipt never claims a publication that did not happen. Both
halves are pinned — the writer may never report bytes accepted for a verdict that
wrote none, and the caller may only abort and fail, never fall through.

**The one step that closes it:** a determinate pre-write refusal in the a729
runtime — a failure that carries `retry_permit: Some(permit)` with
`sink_state_indeterminate: false` when the sink refused before writing a byte.
The caller can then drive it on the same 20-attempt/50 ms budget every other
terminal write uses. That is an API change in `lanes/fired_event_popups_live`,
not in `attach_log.rs`.

That same change is what would let the caller's behaviour be tested through an
executable seam rather than by source matching, which is the reviewer's remaining
note on this residual: `custody.publish` takes a concrete `CaptureTransport` and
a live custody reservation, so today the only way to state "a D75 publication
failure aborts the custody and fails the worker" is structurally.

## Mutation battery

`terminal_write_path_mutations.py` at the worktree root. 24 mutations,
**24 killed, 0 survivors** — `mutation_battery.log`.

One survival is worth recording because it maps the limit of these tests. An
earlier T16 shadowed the real not-written arm with an unreachable
`_not_published => {}` placed ahead of it, and survived: a structural assertion
reads shape, not reachability, so it cannot see an arm that is present but dead.
What stops that mutation is not this battery — rustc's `unreachable_patterns`
warning fires on it, and the mutation had to add `#[allow(unreachable_patterns)]`
to compile quietly at all. T16 now expresses the same silence in a form that IS
watched: the arm stays reachable and stops saying anything.

It reverts via `git checkout -- attach_log.rs`, so uncommitted production edits
are destroyed. Commit first. A full pass takes about 16 minutes (one collector
rebuild per mutation).

## Suite

Run as the `Final` mode nextest step for the Extractor workspace
(`cargo nextest run --locked --workspace --all-targets --no-fail-fast`) with
`CARGO_TARGET_DIR` inside this worktree.

| | tests run | passed | failed | skipped |
| --- | --- | --- | --- | --- |
| base `16640b81` | 6442 | 6434 | 8 | 9 |
| lane HEAD | 6453 | 6445 | 8 | 9 |

Same 8 pre-existing failures, +11 new tests, zero new failures.
Logs: `base_16640b81_extractor_nextest.log`, `head_extractor_nextest.log`.

`-Mode Final` itself cannot run at base: its pre-flight world-fold citation
authority check fails on an unrelated, pre-existing drift
(`d11.rs: declared lines 13161-13802, actual 13186-13827`). Not this lane's, and
not touched by it.
