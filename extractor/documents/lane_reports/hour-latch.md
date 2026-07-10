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
