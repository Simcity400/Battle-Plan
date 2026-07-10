# lane/hour-latch — zero-skip hour-boundary latch (2026-07-10)

Branch: `lane/hour-latch` (base `779d65a17`). Files: `collector/src/hour_latch.rs` (new),
`collector/src/lib.rs`, `recorder_host/src/main.rs`. Fold path (`world_fold.rs`,
`stream_fold.rs`, `compact_contract`) UNTOUCHED. Default **OFF** — legacy sample loop is
byte-identical unless `--hour-latch` is passed.

## The losslessness definition I settled on (load-bearing — read this first)

**This latch provides ZERO-SILENT-GAP ACCOUNTING, not zero data loss.** This scoping is the
honest one and it survived an adversarial Codex review that initially REFUTED the broader
"zero data loss" framing (see §Codex reconciliation).

An async sampler of a sim it may not stop **cannot** record the exact instantaneous world-state
at each hour boundary: the sim advances while we read, so every observation carries a bounded
read-skew, and **intra-hour transients** (a value that changes and reverts inside one game-hour —
e.g. an order issued and cancelled) are unobservable to an *hourly* sampler at all. (A finer or
event-driven sampler could catch them; that is out of scope for this lane and is a separate
architectural item.) Within an explicitly delimited capture interval, assuming a valid monotonic
hour counter, the latch guarantees:

1. **COVERAGE (zero silent gaps).** Every game-hour the sim passes through is *observed and
   accounted* as exactly one of: (a) a capture ATTEMPT (a heavy sweep was authorized for it),
   which the runtime then confirms complete-and-written or leaves as a recorded *failed* attempt;
   or (b) a COALESCED hour with a **specific reason** (superseded / ring-overflow / end-of-session
   flush / reload-epoch-close / anomalous-jump-cap). Formally `observed == attempts + coalesced +
   in_flight` after every operation, and `in_flight == 0` after `flush()`. No hour is ever
   *silently* unaccounted.
2. **BOUNDED, NON-REGRESSED SKEW.** A captured hour's read-skew is of the same KIND as today's
   single-sweep skew (the sweep reads current state; the snapshot is labeled with the hour read at
   sweep start). The latch removes sample-loop dead-time (usually *tightening* skew) but adds a
   small mutex/decision latency (occasionally loosening it), so the skew is **different — usually
   better, occasionally marginally worse — not provably monotonically better.** Bounded by sweep
   duration + that latency, the same order as today.

**A COALESCED hour IS data loss of that hour's per-hour world state.** The latch does not pretend
otherwise; it converts the 20.7 % of hours that were *silently skipped* into explicit,
reason-tagged, bounded accounting. Giving every hour its own faithful snapshot is the job of a
*faster sweep* (Phase 2/4), not of this latch.

## The problem (measured, from the plan doc)

The deep-war run (`runs/live_validation_20260709_221706/stage2_shortspeed5`) skipped **370 of
1790 advanced game-hours (20.7 %)**. Root cause is NOT slow frames per se: the sim's local
inter-hour interval is bursty — p50 129.5 ms but **min 28.2 ms** (~35 game-h/s bursts), p1 33 ms —
while the synchronous sample loop runs the ~150 ms heavy sweep INLINE, so any hour that begins and
ends during a sweep is never noticed. Zero-skip by frame-time alone needs ≤25 ms frames
(unreachable). Avg hour-rate 5.967 h/s vs sweep throughput ~6.6/s (keeps up on average, loses in
dense stretches).

## Design space (A / B / C)

fires once per game-hour on the main thread with state settled. **REJECTED for now:** it patches a
live main-thread SIMULATION function; graded HIGH risk by the Phase-4 scout; needs an MP-checksum
control gate before it could ship; violates the lane's read-only safety posture as a *default*.
The poll-based latch (B) achieves the coverage guarantee without touching the sim, so the detour
is deferred behind a future default-OFF flag with its own MP-checksum non-perturbation gate.

**B — Dedicated high-frequency latch thread (CHOSEN).** A tiny thread polls ONLY the hour counter
(`gs+0x468`) every 2 ms (default; hard-clamped ≤25 ms, well under the 28.2 ms min) via the
existing crash-safe `read_i32_stable` (torn-read guard). It is the **sole authority** for the
observed frontier: it folds the counter VALUE delta (`last+1..=raw`, not edges) so a multi-hour
advance between polls is fully counted. The heavy sweep is latch-triggered (condvar, no
sample-sleep dead-time) and asks the latch which frontier hour to capture. **No game patching,
read-only, cannot mutate the sim.**

**C — Queue / backpressure — brutally honest about what is lossless.** The pending hours live in a
bounded ring (`queue_cap` 64). Two facts decide losslessness under overrun:
- **Momentary bursts (the measured regime): fully absorbed.** The ring buffers the burst; the
  sweep captures the current frontier and coalesces at most the ring-worth of intervening hours,
  draining in the troughs. Coalescing → near-zero because avg throughput permits.
- **Sustained overrun (synthetic 28 ms hours vs 150 ms sweep): NOT recoverable, and honestly so.**
  When hours arrive faster than the sweep drains for long enough, the intermediate hours' full
  world state is **physically unrecoverable** (we cannot rewind the sim). They are COALESCED:
  counted with a reason, never silently dropped. Which hours are recoverable: **only those the
  sweep has throughput to capture** (~ RUN_MS / sweep_ms). The rest are lost-but-accounted. The
  only lossless answers here are (i) a faster sweep (Phase 2/4) or (ii) a cheaper per-hour capture
  — both out of this lane. The latch's job is to make the loss *exact and non-silent*, and to stop
  losing hours to dead-time and to sweep-start latency.

**Faithful labeling (why the sweep never authors the frontier).** The captured hour is always the
latch's own observed frontier; the sweep's independent clock read is used ONLY to label the
emitted snapshot (legacy behavior). The sweep NEVER feeds its counter read into the epoch logic —
that would let a stale sweep read fabricate a false reload and duplicate a captured hour (this was
a real bug Codex found in the first draft; see reconciliation D).

## What was implemented

- `collector/src/hour_latch.rs` — pure, single-threaded `HourLatch` accounting core: `observe`
  (latch-thread authority), `take_capture` (frontier-only), `confirm_capture` (two-phase), `flush`,
  reason-split coalesce counters, generation counter, checked invariant. 16 unit tests.
- `collector/src/lib.rs` — `HourLatchRuntime` (latch thread + condvar + shutdown), wired into the
  compact sample loop behind `hour_latch_enabled`; synchronous initial + final stable reads;
  poison-recovering locks; notify-on-advance; generation-predicate wait; throttled telemetry probe.
- Control flags: `hour_latch_enabled` (default false), `hour_latch_poll_ms` (default 2, clamp
  ≤25), `hour_latch_queue_cap` (default 64). Host flags `--hour-latch [--hour-latch-poll-ms N]
  [--hour-latch-queue-cap N]`, written into `collector_control.json`.
- Telemetry: `hl_*` diagnostic units on the reserved probe domain 250 (same channel as
  `lane_wall`; hosts old & new exclude domain 250 from every coverage/perf gate; the fold ignores
  PERF records). Host MAX-aggregates them and surfaces `compact_stream.hour_latch` with the
  coverage reconciliation (`observed`, `capture_attempts`, `hours_captured`=confirmed,
  `capture_failed`, `hours_coalesced` + 5 reason splits, `max_queue_depth`, `coverage_invariant_ok`,
  `hours_unaccounted`).

## Gates / evidence

- **(a) Synthetic-burst gate — PASS.** `gate_synthetic_burst_28ms_hours_150ms_sweep_zero_missed`:
  28 ms hours, 150 ms sweep, 300 s simulated. Asserts `observed == max_hour+1` (every advanced
  hour observed, none missed), invariant holds every step, `in_flight==0` after flush, and the 5
  coalesce reasons sum to `coalesced`. Under this SUSTAINED overrun most hours coalesce (forced) —
  the test proves they are all **counted with a reason, never silently dropped**.
- **(b) Coverage gate still fail-closes — PASS.** The host per-domain coverage gate and
  `skipped_hours` accounting are UNCHANGED (0 edits); all their tests pass. Latch telemetry is
  additive and excluded from those gates.
- **(c) No fold-path change — structural + green.** `world_fold.rs` / `stream_fold.rs` /
  `compact_contract` have ZERO edits; default-OFF emits no new records; probe domain 250 is
  fold-ignored. The `compact_replay_verification_gates` + `fold_check` suites are green.
- **(d) Build + test — PASS.** `cargo build --release` green; `cargo test` = **1203 passed, 0
  failed** (was 1199 at base; +4 net new tests). 16 `hour_latch` unit tests + 2 host aggregation
  tests included.
- **Cost — measured.** `cost_state_machine_op_is_sub_microsecond` (release): **4.8 ns** per
  observe+take_capture+confirm cycle. At the 2 ms poll the latch thread runs ~500 observe/s; state
  logic ≈ 2.4 µs/s. The poll's real cost is the two `read_i32_stable` reads (~sub-µs each) ⇒
  ~0.05 % of one core, and it adds **zero** per-sweep-frame work except one throttled (every 32nd
  capture) small telemetry record. Does not lean on core count.

## Codex adversarial review — reconciliation (findings verbatim; accept/reject)

Codex (gpt-5.6-sol, xhigh, Mode A read-only) returned **REFUTED/unsafe** on the first draft. Every
finding and my disposition:

- **DEFINITION — REFUTED (overclaim). ACCEPTED.** "A coalesced hour has lost its per-hour world
  state. Counting that loss does not make it lossless… re-scopes zero data loss into zero silent
  counter gaps." → Rewrote the definition to **zero-silent-gap accounting, not zero data loss**
  (module doc + this report + host manifest `note`). Skew claim "only ever tightens" was **false**
  (mutex/decision latency can loosen it) → corrected to "different — usually better, occasionally
  marginally worse." "Unobservable to ANY hourly sampler" softened to *hourly* (a finer/event
  sampler could catch transients).
- **CODE D — REFUTED: stale sweep read fabricates a reload + duplicate capture. ACCEPTED (real
  bug).** The old API took the sweep's single-read counter into `begin_capture`; a stale read of an
  already-captured hour triggered the backward-jump branch, coalesced a real newer hour, and
  re-captured. → Redesigned to `take_capture()` (frontier-only); the sweep never authors epoch
  transitions. Regression test `stale_sweep_read_cannot_fabricate_reload_or_duplicate`.
- **CODE A — REFUTED: counter input not trustworthy. PARTIALLY ACCEPTED.** Accepted: poll period
  is best-effort not real-time; config could reach 1000 ms → **clamped ≤25 ms** in DLL + host;
  fixed the contradictory "<=1 ms" comment; the sweep-fed single read is gone (subsumed by D).
  Documented that on x86-64 an aligned 4-byte read is atomic (tearing is not the real risk;
  `read_i32_stable` guards decommit/partial-copy). The "must PROVE atomicity" demand is over-strict
  for an aligned dword — noted, not treated as a blocker.
- **CODE C — REFUTED: "captured" means attempted. ACCEPTED.** `captured` incremented before the
  sweep ran/succeeded. → Two-phase: `take_capture` records an ATTEMPT; runtime calls
  `confirm_capture` ONLY on a complete written frame. Telemetry now reports `capture_attempts`,
  `hours_captured`(=confirmed), and `capture_failed = attempts − confirmed` as a distinct loss
  class. Re-affirmed that per-hour DATA completeness is enforced by the SEPARATE, unchanged host
  per-domain coverage gate.
- **CODE B — REFUTED: durable loss accounting. PARTIALLY ACCEPTED.** (1) "coalescing is
  aggregate-only" → added 5 reason-split counters (superseded/overflow/flush/backward/anomalous);
  invariant checks they sum to `coalesced`. (2) "any surviving probe yields final totals" was an
  **overclaim** → corrected to "as of the last emitted probe; a hard kill loses only the in-flight
  tail, bounded by ring depth, which flush would have coalesced." (3) Saturating counters → switched
  the invariant to **checked** math (saturation surfaces as `false`, never a silent wrong `true`).
  **REJECTED-as-out-of-scope but DOCUMENTED:** `write_sinks`/`write_handle` silently discard
  pipe-write errors — a real, PRE-EXISTING collector-wide behavior (every record, not just my
  telemetry, rides the same sink). Fixing the sink error-handling is a cross-cutting change beyond
  this lane and the one-fix-round cap; flagged for a dedicated lane.
- **CODE E — REFUTED overall. MOSTLY ACCEPTED.** Fixed: latch-thread + `wait_for_hour` now recover
  a poisoned mutex (were `if let Ok`); condvar wait uses a **generation predicate** (no lost
  wakeup); shutdown takes a **final synchronous stable reading** before flush (an hour advancing
  after the last poll is now counted). **DOCUMENTED-as-inherited:** `date_addr` is resolved once and
  not re-resolved on reload — this is the whole collector's existing session-stable-pointer
  assumption (every domain reader shares it), not specific to this lane.
- **CODE F — REFUTED: overhead not shown negligible. ACCEPTED.** Now notify **only on frontier
  advance** (not every idle poll); telemetry probe **throttled** to every 32nd confirmed capture +
  final; added the 4.8 ns/cycle measurement. Corrected "zero per-frame work" to "one throttled
  small record per 32 captures."

**Review independence:** per the lane brief, I do not get the final word on findings against my own
definition. This is a lossless-adjacent diff; it warrants the orchestrator's independent read-only
final gate on the post-fix state (I did ONE fix round, per the escalation cap, then stopped).

## What must still be validated LIVE (offline is done; these need the game)

1. **Live shadow-soak** on a deep-war save at speed 5 (bursty regime): with `--hour-latch` on,
   assert `hours_observed` matches the counter's advanced range 1:1 (no missed hour), the coverage
   invariant holds, and coalescing is near-zero in the momentary-burst regime.
2. **Paired A/B vs legacy** (latch off vs on) on the same live save: skipped-hour count,
   frame avg/p95, `max_queue_depth`, host peak RAM. Expectation: the latch converts the 20.7 %
   silent skips into ~0 dead-time loss + exact coalesce accounting; it does NOT raise heavy-domain
   snapshot throughput beyond the sweep's speed (that's Phase 2/4).
3. **6-core portability companion** (G3): the latch thread must not starve the game on a weaker host.
4. **(If the detour A is ever pursued)** MP-checksum non-perturbation control gate before shipping.

## Independent adversarial gate — RUN 2026-07-10, VERDICT: SOUND

Run by an Opus reviewer under a temporary per-instance user authorization (Codex
rate-limited; authorization revoked later the same day — the Codex Mode-A re-stamp in
`CODEX_DISPATCH_QUEUE.md` remains REQUIRED before enabling). Reviewer was independent of the
implementer. Summary of the verdict (full attack list ran: i32 edges, i64-delta wraparound,
backward jumps, raw==last, first-observe seeding, shutdown/flush ordering, TOCTOU on last,
counter overflow, OFF-path byte identity):

- All three claimed guarantees held. OFF-path: every latch touchpoint gated on
  `hour_latch_runtime.as_ref()`, no thread, no reads, no records. Invariant
  `observed == attempts + coalesced + in_flight` traced through every mutator; multi-hour
  jumps delta-folded (`last+1..=raw`) with the >768-hours/poll remainder booked as
  `coalesced_anomalous` — accounted, never silent. Shutdown joins the thread BEFORE the final
  observe+flush+telemetry under one lock — no post-flush in_flight, no cross-flush
  double-count. The old stale-sweep false-reload/duplicate bug is structurally dead (the API
  removed the sweep's counter argument; `observe` is the sole author of `last_raw`).
- MINOR (2): (a) `decide()` books a capture attempt even when `clock_sample` is None (no
  sweep runs) — stands as an accounted failed attempt, same emit outcome as legacy on an
  unreadable clock, but arguably deserves a dedicated "no-clock" reason tag. (b) The emitted
  snapshot's hour label (from `clock_sample`) can differ from the accounting frontier hour —
  the disclosed BOUNDED-SKEW property, same class as the legacy single-sweep skew.

## Codex re-stamp — RUN 2026-07-10 (post-quota), VERDICT: REFUTED — adjudicated: REAL FIXES REQUIRED

The Codex Mode-A gate (required by user directive) REFUTED, contradicting the earlier Opus
SOUND verdict. Orchestrator adjudication of the 8 findings:

**REQUIRED FIXES (block any enable; fix lane queued in CODEX_DISPATCH_QUEUE.md):**
1. **Label/frontier mismatch (SILENT-DATA-LOSS, the big one).** The capture decision consumes
   the latch frontier hour but the emitted frame is labeled from the separately-read sweep
   clock. Stale clock ⇒ a duplicate frame labeled with the PREVIOUS hour and NO frame labeled
   with the consumed frontier hour — accounting says attempt+confirmed, the folded output has
   a silent gap. (Opus saw the same mechanism and under-weighted it as bounded skew; Codex's
   duplicate scenario is concrete and correct.) Fix: label the frame with the frontier hour
   from take_capture, or fail the attempt on label/frontier disagreement.
2. **confirm() on failure/partial frames (CORRECTNESS).** produced=true on any nonempty record
   vector; failure flags never inspected — a PARTIAL frame is booked confirmed-complete.
   Fix: confirm only when status ∩ FAILURE_MASK (minus declared tolerances) is empty; else
   book recorded-failed.
3. **Terminal read-failure window (SILENT-DATA-LOSS).** If every poll from some point through
   shutdown fails, hours advanced in that window are unaccounted while the invariant still
   passes. Fix: a terminal marker (last_observe_failed + read_failures) that downgrades the
   run's accounting claim from COMPLETE to TRUNCATED-AT.
4. **Fail-open on thread-spawn failure (SILENT-DATA-LOSS).** start() returning None silently
   falls back to the legacy lossy loop with hour_latch_enabled=true. Fix: telemetry marker +
   manifest flag (latch_requested_but_inactive).
5. **Sink-write failure ⇒ false confirm** — real, but the error-swallowing write path is
   PRE-EXISTING and collector-wide, not latch-specific; scope in the fix lane: either plumb
   write results into confirm() or rename the counter to emitted (claim only what is true).

**ADJUDICATED NOT CODE DEFECTS:**
6. Reload excursion fully between two polls (advance 101..150 then reload to 100) is
   UNOBSERVABLE by any polling sampler — no fix exists short of hooking the engine (rejected:
   MP-checksum risk). Contract wording fix: the guarantee covers OBSERVED advances; the
   ≤25 ms poll clamp bounds the blind window.
7. u64 saturation masking (MINOR): needs ~5.8e19 hours; unreachable.
8. i32::MAX→MIN wrap misread as reload (MINOR): game-hour raw magnitudes never approach
   i32::MAX in any real campaign.

Status: **NOT CLEARED, now with required fixes** (it was never cleared — default OFF
unchanged). Re-gate (fresh Codex Mode-A) required after the fix lane, then live gates.

Meta: reviewer disagreement (Opus SOUND vs Codex REFUTED) is itself evidence for keeping
adversarial gates model-strong; findings 1–4 are real and Opus missed or under-weighted them.

## Fix lane 0aF (2026-07-10)

Mission: fix required findings 1-5 from the final Codex re-stamp without changing the fold
path or the default-OFF stream. Branch remained `lane/hour-latch-fix`; no push was performed.

### Fixes and regression coverage

1. **Frontier-hour labeling fixed.** When the latch is active,
   `compact_sweep_game_hour` now takes the hour directly from
   `CaptureDecision::Capture { hour, .. }`; that exact value is passed into every compact
   record in the frame. The independently-read sweep clock is used only by the unchanged
   legacy/default-OFF branch. The completeness gate additionally rejects any compact record whose
   header label differs from the consumed frontier
   (`collector/src/lib.rs:1306-1325,12920-12996`).
   Regression: `hour_latch_frontier_labels_frame_when_sweep_clock_is_stale` emits hour 100,
   leaves the sweep clock stale at 100, observes/consumes 101, and asserts encoded labels
   `[100, 101]`, no stale duplicate, no failed attempt, and the accounting invariant
   (`collector/src/lib.rs:55547-55621`).

2. **Status-complete gating fixed.** A nonempty vector is no longer sufficient. The latch walks
   the newly-built H4CR frame, reads every domain PERF status, applies
   `compact_status_failure_flags_after_expected_partial_tolerance(domain_id, flags)`, checks the
   frame-end flags, framing, one terminal frame-end, and the expected hour label, and fails closed
   on any remaining member of `COMPACT_STATUS_FAILURE_MASK`
   (`collector/src/lib.rs:12939-12998,1322-1339`). Regression:
   `hour_latch_partial_frame_is_attempted_but_not_recorded_complete` proves a non-tolerated
   PARTIAL produces `attempts=1, emitted_to_sink=0, capture_failed=1`; companion
   `hour_latch_declared_partial_tolerance_remains_emit_eligible` proves declared domain-53
   tolerance is not over-rejected (`collector/src/lib.rs:55624-55684`).

3. **Terminal observation failure surfaced.** `last_observe_failed` is set by a failed poll and
   cleared only by a later good `observe`. The final synchronous shutdown read therefore leaves
   an exact terminal marker without inventing unobserved hours
   (`collector/src/hour_latch.rs:140-151,269-287,446-451`). The host aggregates this one marker
   by last value (all cumulative counters remain MAX) and emits
   `accounting_claim=TRUNCATED-AT-LAST-GOOD-OBSERVE` while the arithmetic invariant can remain
   true (`recorder_host/src/main.rs:16484-16524,16646-16655`). Regressions:
   `failing_final_polls_mark_claim_truncated_without_breaking_invariant`,
   `later_good_observe_clears_terminal_failure_marker`, and
   `hour_latch_terminal_read_failure_downgrades_complete_claim`.

4. **Spawn failure is fail-closed in telemetry.** Activation records whether the latch was
   requested and whether a runtime actually became active. Any requested-but-inactive compact run
   keeps the legacy fallback but emits a final `hl_latch_requested_but_inactive=1` probe; the
   manifest reports `enabled=false`, `latch_requested_but_inactive=true`, and
   `accounting_claim=INACTIVE` (`collector/src/lib.rs:1263-1280,1543-1565,13262-13267`;
   `recorder_host/src/main.rs:16484-16524`). Regression:
   `hour_latch_spawn_failure_sets_requested_but_inactive_signal` injects a deterministic thread
   spawn error; `hour_latch_requested_but_inactive_is_fail_closed_in_manifest` proves the host
   claim is not COMPLETE.

5. **Sink claim narrowed rather than widening collector-wide error handling.** This lane chose the
   smaller honest option: `confirmed` was renamed to `emitted_to_sink`, and it increments only
   after a status-complete frame is handed to `write_sinks`
   (`collector/src/hour_latch.rs:101-109,170-205,402-415`;
   `collector/src/lib.rs:1328-1339`). The manifest exposes
   `frames_emitted_to_sink`, `recorded_failed`, `sink_write_success_verified=false`, and the
   explicit gap that `write_sinks/write_handle` do not report errors
   (`recorder_host/src/main.rs:16466-16512`). This does not claim durable sink commit and does not
   rewrite the pre-existing collector-wide writer.

Default OFF remains structurally isolated: when `hour_latch_enabled=false`,
`hour_latch_requested=false`, no runtime/thread starts, no latch read occurs, no latch probe is
emitted, and `compact_sweep_game_hour(None, ...)` uses the legacy sweep-clock label. Fold files
`world_fold.rs`, `stream_fold.rs`, and `compact_contract` have zero diff.

### Gate results (run from `extractor/native`)

**New regressions, deterministic run 1 and run 2:**

```text
cargo test -p collector hour_latch
run 1: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 324 filtered out; finished in 0.04s
run 2: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 324 filtered out; finished in 0.04s

cargo test -p recorder_host hour_latch
run 1: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 184 filtered out; finished in 0.00s
run 2: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 184 filtered out; finished in 0.00s
```

**Required release suite:**

```text
cargo test --release --no-fail-fast
exit_code=0
test result: ok. 340 passed; 0 failed; 5 ignored; 0 measured; 0 filtered out; finished in 0.07s
test extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build ... ok
test result: ok. 113 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 173 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 186 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.13s
```

Cargo emitted 89 `test result:` lines; mechanical aggregate: **1223 passed, 0 failed,
8 ignored**. The known diplomacy timing flake passed on this run; there were no failures.

**G1 fold identity (exact requested 600 MiB prefix):**

```text
exit_code=0 prefix_size=629145600 deleted=true
"chain_hash": "bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7",
"bytes_folded": 622604576,
"hours_emitted": 198,
"records_total": 22059,
```

The source corpus was read in place. `extractor/native/runs/tmp_g1_prefix.bin` was deleted and
verified absent. Post-cleanup free space: `143591657472` bytes.

**Fold-path structural check:**

```text
git diff --exit-code -- extractor/native/recorder_host/src/world_fold.rs extractor/native/recorder_host/src/bin/stream_fold.rs extractor/native/compact_contract
fold_diff_exit=0 output=""
```

Files changed by this fix lane: `collector/src/hour_latch.rs`, latch-owned sections of
`collector/src/lib.rs`, `recorder_host/src/main.rs`, and this lane report. No live game gate
was run. Per workflow, the implementer did not perform the independent adversarial re-gate; that
remains the orchestrator's next step.

### Commit status

The required local commit was attempted after all gates, but the environment denied creation of
the shared-worktree index lock:

```text
fatal: Unable to create 'C:/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/mod/battle-plan/.git/worktrees/hoi4_lane_hour_latch_fix/index.lock': Permission denied
```

No files were staged and no push was attempted. The verified four-file diff remains in the lane
worktree for the orchestrator to commit once Git metadata is writable.

## Gate 0a2 on the 0aF fixes — REFUTED (2026-07-10); one fix round owed, lane HELD UNMERGED

Three findings, all accepted as real (verbatim verdict in the orchestrator session):
1. SILENT-DATA-LOSS — `--hour-latch` with a non-compact (JSON) format is accepted by the CLI
   but the collector defines "requested" as enabled&&Compact, so the latch is silently absent
   with NO latch_requested_but_inactive marker. Fix: host rejects or warns on the combination,
   or "requested" = enabled alone.
2. CORRECTNESS — compact_frame_complete_for_hour_latch runs on EVERY legacy compact frame
   before the runtime-exists check: latch-specific record parsing on the OFF path (labels and
   records unchanged, but cadence work added). Fix: gate the helper on runtime presence.
3. CORRECTNESS — the host's legacy-compat mapping feeds pre-fix hl_confirmed (which counted
   non-tolerated PARTIAL frames) into emitted_to_sink, whose new contract implies
   status-completeness. Fix: mark legacy-sourced values as legacy_semantics in the manifest.

Next session: one Codex fix round (Mode C, this worktree ../hoi4_lane_hour_latch_fix, branch
lane/hour-latch-fix @ 50c50f804), then a fresh Mode-A gate; merge only on SOUND. Then live
gates (shadow-soak + paired A/B + 6-core) before any enable.

## Fix round 2 (0aF2) (2026-07-10)

Mission: implement exactly the three accepted Gate 0a2 findings on branch
`lane/hour-latch-fix` at `6d109ff78`, without touching the fold path or the default-OFF
stream. No commit or push was performed.

### Fixes and regression coverage

1. **Non-compact latch requests now fail loudly in the host.** After profile/explicit-format
   resolution, `parse_args` rejects `--hour-latch` unless the effective stream format is
   compact, so neither the implicit JSON default nor explicit `--stream-format json` can
   silently discard the request (`recorder_host/src/main.rs:1816-1822`). Regression:
   `parse_args_rejects_hour_latch_with_non_compact_stream` covers implicit JSON, explicit JSON,
   and the accepted compact combination (`recorder_host/src/main.rs:27512-27545`).

2. **Latch frame-completeness work is gated on runtime presence.** The
   `compact_frame_complete_for_hour_latch` parser is now inside a lazy runtime-presence gate;
   legacy/default-OFF compact frames are written through the unchanged path without running
   latch record parsing (`collector/src/lib.rs:1323-1340,13006-13017`). Regression:
   `hour_latch_off_path_does_not_run_frame_completeness_gate` supplies no runtime and proves the
   active closure is never invoked (`collector/src/lib.rs:55706-55718`).

3. **Legacy `hl_confirmed` is no longer promoted into the status-complete contract.** New
   `hl_emitted_to_sink` remains the only source for `frames_emitted_to_sink`, `capture_failed`,
   and `recorded_failed`. A pre-fix stream carrying only `hl_confirmed` now reports
   `emission_semantics="legacy_semantics"`, `legacy_semantics=true`, and its count under
   `legacy_confirmed`; the three status-complete-derived fields are null
   (`recorder_host/src/main.rs:16477-16536`). Regression:
   `hour_latch_legacy_confirmed_is_marked_legacy_not_status_complete` proves the split
   (`recorder_host/src/main.rs:17639-17663`).

Default OFF remains byte-identical and no longer performs latch frame parsing. Fold files
`world_fold.rs`, `stream_fold.rs`, and `compact_contract` have zero textual diff.

### Gate results (run from `extractor/native`)

**Targeted regressions:**

```text
cargo test -p collector hour_latch
exit_code=0
test attach_log::tests::hour_latch_off_path_does_not_run_frame_completeness_gate ... ok
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 324 filtered out; finished in 0.06s

cargo test -p recorder_host hour_latch
exit_code=0
test tests::hour_latch_legacy_confirmed_is_marked_legacy_not_status_complete ... ok
test tests::parse_args_rejects_hour_latch_with_non_compact_stream ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 184 filtered out; finished in 0.02s
```

**Required release suite:**

```text
cargo test --release --no-fail-fast
exit_code=0
test_result_lines=89
aggregate={"passed":1226,"failed":0,"ignored":8}
test attach_log::tests::hour_latch_off_path_does_not_run_frame_completeness_gate ... ok
test extractor::tests::payload_phase_times_populated_and_bounded_by_outer_build ... ok
test tests::hour_latch_legacy_confirmed_is_marked_legacy_not_status_complete ... ok
test tests::parse_args_rejects_hour_latch_with_non_compact_stream ... ok
test result: ok. 341 passed; 0 failed; 5 ignored; 0 measured; 0 filtered out; finished in 0.07s
test result: ok. 113 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 173 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 188 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.14s
```

Cargo emitted 89 `test result:` lines; mechanical aggregate: **1226 passed, 0 failed,
8 ignored**. The known diplomacy timing test passed; there were no failures.

**G1 fold identity (exact requested 600 MiB prefix):**

```text
exit_code=0 prefix_size=629145600 deleted=true
"chain_hash": "bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7",
"bytes_folded": 622604576,
"hours_emitted": 198,
"records_total": 22059,
"stream_bytes": 629145600,
"truncated_tail_bytes": 6541024,
```

The source corpus was read in place from the main worktree. The lane prefix
`extractor/native/runs/tmp_g1_prefix.bin` was deleted and verified absent. Post-cleanup free
space: `136629805056` bytes.

**Fold-path structural check:**

```text
git diff --exit-code -- extractor/native/recorder_host/src/world_fold.rs extractor/native/recorder_host/src/bin/stream_fold.rs extractor/native/compact_contract
fold_diff_exit=0 output=""
```

Files changed by 0aF2: `collector/src/lib.rs`, `recorder_host/src/main.rs`, and this lane
report. Per workflow, the implementer did not perform the independent adversarial re-gate; that
remains the orchestrator's next step.

## Gate 0a3 — REFUTED on one residual finding; orchestrator direct fix (2026-07-10)

0a3 verified fixes 2+3 and the host CLI rejection, but found the collector still defined
"requested" as enabled&&Compact — a directly-authored control file (alternate writers bypass
the CLI) with hour_latch_enabled=true + non-compact format silently fell back with no marker.
Direct fix (fix-round cap spent): requested = enabled ALONE; activation additionally requires
Compact; latch_requested_but_inactive now fires for that combination. collector 341/0.
REMAINING before merge: gate 0a4 on this final state (next session), then live gates.

## Gate 0a4 — REFUTED on one defect introduced by the 0a3 direct fix; fixed (2026-07-10)

0a4 confirmed the requested-alone semantics but found the inactive-marker PROBE RECORD (H4CR
compact framing) was appended unconditionally — onto a JSON-format stream that is stream
corruption, not a marker. Fix: the compact probe record is now compact-stream-exclusive
(.filter on stream_format == Compact); on non-compact formats requested-but-inactive is
carried by telemetry/manifest only. collector suite green post-fix. NEXT: gate 0a5 on this
final state, then merge on SOUND, then live gates.
