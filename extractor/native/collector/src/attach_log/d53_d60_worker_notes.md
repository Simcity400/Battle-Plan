# D53 / D60 off-frame worker paths — traced state (lane d53_d60_worker)

Base: `integration/extractor-recovery-20260731` @ `4d0e0dd2`.
All line numbers are `extractor/native/collector/src/attach_log.rs` at this
lane's HEAD unless stated otherwise.

## 1. The traced read paths

### D53 — `read_compact_diplomacy_intel_operatives_live` (:48211)

1. `:48221` inherited-failure short circuit → `SO_D53_PARTIAL_INHERITED`.
2. `:48277` `domain53_scan_needles(module_base)` → six vtable needles; unresolved
   or `>usize` → `domain53_needles_unresolved` / `domain53_needle_overflow`.
3. `:48306` `heap_sweep_register_needles` + `heap_sweep_ensure_started` +
   `heap_sweep_snapshot()`. Needle order: `[operative, war_gregorian,
   war_game_date, operation_instance, strategic_operative, intel_network]`.
   The six are ALSO registered at attach by
   `register_shared_heap_sweep_attach_needles` (:16935), before
   `heap_sweep_ensure_started`, so they make sweep pass 1.
4. `:48324` reap step (a): `handle.is_finished()` → `handle.join()` →
   `domain53_apply_war_validation_outcome`.
5. `:48342` spawn step (b): `domain53_war_validation_should_spawn` +
   both war needles present in the snapshot → `domain53_arm_war_validation_attempt`
   → `thread::spawn(domain53_validate_war_row_addrs)`.
6. `:48408` on-frame discovery rebuild, gen-gated by
   `heap_sweep_should_rebuild`; any of the four small needles absent from the
   last completed pass → `domain53_discovery_cold`.
7. `:48446` consume: `tracker.discovery` → `domain53_discovery_not_warmed`;
   `tracker.validated_war_addrs` → `domain53_war_addrs_unavailable_reason`.
8. `:48470` `extract_rows_from_current_roots_with_validated_war_addrs`.
9. Frame-level unwind is caught at `:36343` →
   `compact_diplomacy_intel_operatives_worker_panic_read` (`SO_D53_WORKER_PANIC`).
   That guard covers the READ thread, NOT the war-validation worker.

### D60 — `read_compact_capitulation_war_state_live` (:49964)

Same shape: needles (`capwar_war_date_vtables`, module_base + RVA, so never 0)
→ register/ensure/snapshot → reap (a) → spawn (b), gated additionally on both
needles being covered → epoch guard (:50147) → consume (:50156) →
`extract_frame_with_discovery`. D60's off-frame worker is
`discover_war_rows_from_relation_roots`; the sweep hits are only a readiness
witness, deliberately not consumed.

## 2. Verdict on "the worker never returns"

**Refuted as stated; the real defect was that the worker's RESULT never
reached the record.** The worker returns normally. On the base tip the D53
reap arm was:

```rust
Ok(Ok(_)) | Ok(Err(_)) | Err(_) => { tracker.stale_frames += 1; }
```

`stale_frames` is written at five sites and read by NO production code
(`grep stale_frames` — the only readers are tests). So:

* a completed `Err(ExtractError)` and a worker unwind were both erased;
* the consume side reported the hardcoded `domain53_war_addrs_cold`, i.e.
  "no worker has run for this generation" — false for a worker that ran and
  failed. That is a fabricated typed reason on a fail-closed path;
* `war_attempted_gen` stayed pinned, and the base spawn gate
  (`offframe_validation_should_spawn`) has no re-arm, so no retry was possible
  until the shared sweep published a NEWER generation.

Scale of the strand: `heap_sweep_idle_interval` defaults to 30 s and a pass is
a full private-heap scan. The in-file measurement at :2583 records that on
`runs/20260731-deflist-16` generation 1 was current for 41.7 s of a ~57 s
capture. One transient failure therefore strands d53 for effectively the whole
capture — the same failure mode D60's tracker comment (:18455) already records
for itself ("one transient root/torn-read failure during warm-up permanently
stranded D60 at `capwar_sweep_not_warmed` for the entire capture"). D60 got the
fix; D53 never did.

## 3. Can a scheduled read complete?

* **Live game, warm snapshot: YES on this tip.** Attach registers every needle
  before `heap_sweep_ensure_started`, the sweep settles 250 ms and publishes
  generation 1 with all needles present; the first frame at gen>=1 spawns the
  worker and rebuilds discovery; a later frame reaps and adopts; from then on
  the consume path runs. Nothing structurally blocks completion. The
  precondition is that at least one full sweep pass completes inside the
  capture window AND the worker succeeds once. Not proven live by this lane —
  see §5.
* **Cold snapshot (generation 0): fails closed, correctly.** D53 →
  `domain53_discovery_cold`; D60 → `capwar_sweep_not_warmed`. Both emit
  `SOURCE_UNAVAILABLE | PARTIAL_DATA` with an empty payload.
* **Offline (no live hoi4.exe): cannot complete, by construction.** The worker
  calls `resolve_domain53_frame_roots` / `resolve_frame_roots` against process
  memory; with no game roots it returns `Err`. Post-fix the record now names
  that error instead of claiming "cold".

## 4. What was changed

See commits `D53: the war-row worker returns — its result was thrown away`,
`D53 worker tests: close the adopt-arm test that could not fail`, and
`Review round 1: cover the production wiring and the single-flight guards`.
Eleven named tests; each is proved to redden under an applied-then-reverted
mutation — the transcript is `runs/d53_d60_worker/mutations.txt` (gitignored;
the mutation list is reproduced in the lane report).

Three behavioural changes, not one. The first commit message said "the only
change is that it now names the truth"; that is wrong and is corrected here.

1. Truthful failure attribution on the fail-closed path (the headline fix).
2. SCHEDULING: the bounded same-generation re-arm lets d53 run another worker
   after backoff and succeed within the same sweep generation, where the base
   stayed stranded until a newer generation appeared.
3. CLOCK SAMPLING: `domain53_refresh_war_validation` evaluates the reap's
   backoff stamp and the spawn gate against ONE `Instant`, where the inline
   block sampled twice. The two old samples were separated only by
   `handle.join()` on an already-finished thread, so the difference is
   microseconds against a 500 ms–5 s backoff; the visible consequence is that
   a deadline crossing inside the helper is taken on the next frame (~125 ms)
   instead of this one. Commit eaecb18b called the extraction "pure"; it is
   not, and this is the difference. (Codex review 019fbba9.)

No validation was weakened and no failed read became clean.

### Coverage boundary of the tests

The tests cover the production STAGE IMPLEMENTATION, not the reader's
invocation of it. `domain53_refresh_war_validation` is the frame path's whole
reap/spawn stage and the tests execute that exact function — but deleting the
reader's CALL to it (attach_log.rs :48323) would still leave them green.
NOT covered: the reader's own composition
(`read_compact_diplomacy_intel_operatives_live`) — needle resolution, the
consume site's choice of `domain53_war_addrs_unavailable_reason`, and the
hand-off of that reason into `domain53_source_unavailable_read`. Reaching those
needs a live `CollectorState` and the real multi-gigabyte sweep service running
inside the test process, so re-hardcoding the reason at the consume site would
still pass. See §5 item 6.

## 5. Open items this lane did NOT close

1. **No live proof.** No hoi4.exe capture was run, so "D53 now emits a
   DOMAIN_DATA record" is unproven end to end. One more step closes it: a
   single live capture with `BATTLE_PLAN_HEAP_SWEEP_IDLE_SECS=5`, long enough for two
   sweep generations, checking the d53/d60 records for
   `domain53_war_addrs_pending` followed by a clean frame.
2. **D53 has no sweep-EPOCH guard** where D60 does (:50147,
   `capwar_sweep_epoch_changed`). After a sweep-service death/restart D53 keeps
   consuming a `validated_war_addrs` set validated under the dead epoch for the
   generation between the new epoch's first pass and its first adoption.
   Deliberately not changed: D53 already tolerates cross-generation staleness
   by design, so whether epoch death differs from ordinary staleness is a
   design decision, not a defect with one right answer.
3. **`stale_frames` is dead for both domains** (superseded-success counts for
   D53/D60, plus D60's degraded-discovery frames). Surfacing it needs a new
   `Domain53Diag` field, which is diagnostic-channel surface owned elsewhere.
4. **`heap_sweep_ensure_started` (:102106) treats a FAILED `thread::spawn` as
   "not running"**: `.spawn(..).ok()` stores `None`, `running` is then false on
   every subsequent call, and each reap/restart resets the published snapshot
   to cold (:102135). Under sustained spawn failure no generation is ever
   published and every sweep consumer — not just d53/d60 — stays cold forever.
   Shared-service change; reported, not fixed here.
5. **A background-worker failure is invisible on a d53 SUCCESS frame.** D60
   reports `capwar_discovery_pending` / its failure reason on its success
   return (:50230); D53's success return carries `failure_reason: None`.
   Adding one there would be wrong as the struct stands:
   `diag.failure_reason.is_none()` is used at :47926/:47994/:48079 as the
   "this frame succeeded" predicate that gates the phase/seg/relflag
   diagnostic units, so a background-worker reason would silently suppress
   them. Needs a separate diag field — diagnostic-channel surface.
6. **The reader's own composition is untested** (see §4). No test calls
   `read_compact_diplomacy_intel_operatives_live`, so a regression that
   re-hardcodes the consume-site reason, or that stops passing the war-worker
   panic reason into `domain53_source_unavailable_read`, would stay green. One
   more step closes it: an injected seam on the reader for the sweep snapshot
   and the memory impl, so the whole reader can run in a unit test without
   starting the sweep service. That is a structural change to a 300-line
   reader on the frame path and was not attempted here.

## 6. Shared-sweep restart policy (lane sweep_resilience)

Closes §5 item 4. `heap_sweep_ensure_started` used `.spawn(..).ok()`, so a
FAILED `thread::spawn` stored a `None` handle. `running` was then false on
every subsequent call, and each call reaped the threadless service and reset
the published snapshot to cold (`SweepSnapshot::empty()`). Six call sites
reach `heap_sweep_ensure_started` (two at attach, four consumer frame paths), so
under sustained spawn failure the reset ran many times per frame, no generation
was ever published, every sweep consumer stayed cold for the whole capture, and
the only wire evidence was each consumer's ORDINARY cold reason. Nothing
distinguished "the first pass has not finished" from "the thread was never
created".

Three changes:

* `HeapSweepService.handle` is no longer `Option`. A service exists only when a
  thread was really created, so a threadless service — the state the whole
  defect lived in — is unconstructible, and a failed start cannot re-cold
  anything because it stores nothing to reap.
* `HeapSweepSpawnGovernor` retains the failure count, the consecutive streak,
  the last OS error and a retry deadline, under the same mutex as the service
  slot. The deadline uses the shared `offframe_validation_retry_delay` curve
  (500 ms .. 5 s), so restarts are bounded instead of per-call.
* `heap_sweep_diagnostic_units` emits `heap_sweep_spawn_failures` and
  `heap_sweep_spawn_last_os_error` (plus the epoch/generation it already
  emitted when degraded). A healthy service still emits nothing.

Not a fail-open change. While the service is down its consumers stay cold, and
that remains the correct outcome; what was wrong was the silence and the
snapshot thrash, and only those were changed. `heap_sweep_ensure_started_in`
takes the spawn and the snapshot reset as injected effects so the policy is
testable without a real multi-gigabyte scan inside the test process.

## 7. Why d53 needs no sweep-EPOCH guard

Closes §5 item 2. The answer is not symmetry with d60 — it is an invariant of
the service:

1. A new epoch is only ever allocated inside `heap_sweep_ensure_started_in`,
   and every path that reaches it either reaps a service whose thread really
   ran (which calls `heap_sweep_reset_published_snapshot`) or starts from a
   cell that never held a thread (whose snapshot is already cold). `EPOCH
   CHANGE implies COLD PUBLISHED SNAPSHOT` therefore holds on every path.
   `heap_sweep_shutdown` resets it as well.
2. `SweepSnapshot::empty()` has `generation == 0` and returns `None` from
   `hits_for` for every needle.
3. d53's operative-side rebuild gate is `heap_sweep_should_rebuild`, i.e.
   `last_consumed_gen != Some(snapshot_gen)` — INEQUALITY, not `>`. With
   `tracker.sweep_gen == Some(N)` and a cold snapshot at generation 0 the gate
   is TRUE, so the frame re-enters the rebuild, finds the first operative
   needle uncovered, and returns `domain53_discovery_cold` BEFORE
   `tracker.sweep_gen` is reassigned. Every frame of the dead-epoch window
   takes that same path.

So d53 already fails closed for the entire dead-epoch window, while still
holding the dead epoch's discovery and validated war addresses. Adding a
`validated_epoch` check would change the reason name and nothing else.

What about the frames AFTER the new epoch's first pass publishes? d53 rebuilds
discovery at the new generation and, for one or more frames, still serves the
war-address set validated under the DEAD epoch. That is not a new exposure: it
is the same cross-generation retention d53 performs at every ordinary
generation change, with the same bound (one sweep pass plus one worker
round-trip), and the addresses are absolute heap addresses that a sweep restart
does not invalidate — every one is re-read and re-validated per frame inside
`extract_rows_from_current_roots_with_validated_war_addrs`. Its provenance is
already on the wire as `domain53_war_validated_generation` next to
`domain53_sweep_generation`.

d60 is different because it has no equivalent on-frame gate. Its `discovery` is
produced entirely by the off-frame worker and consumed regardless of the
published generation, so without the explicit `validated_epoch` check it would
keep serving a dead epoch's war set on CLEAN frames indefinitely. Its guard
stays.

The invariant this rests on is pinned by
`heap_sweep_reaping_a_real_thread_still_recolds_the_snapshot` and the
conclusion by `d53_fails_closed_for_the_whole_dead_sweep_epoch_window`.

## 8. `stale_frames` and the success-frame worker note

Closes §5 items 3 and 5.

`stale_frames` now reaches the wire in both domains:
`domain53_sweep_stale_frames` (new `Domain53Diag` field) and
`capwar60_sweep_stale_frames` (new `CompactCapitulationWarStateLiveRead` field,
carried from the tracker on every d60 return). Both are omitted when zero, so a
clean capture is unchanged. The d53 field is stamped from the tracker on every
return including the success one; the `domain53_frame_degraded` arm re-stamps
it after its own increment so that frame counts itself.

For the success-frame worker cause, §5 item 5 was right that
`diag.failure_reason` cannot carry it. Two separate consumers key off it:
`failure_reason.is_none()` gates the phase/seg/relflag blocks, and
`domain53_source_unavailable_read` lifts `COMPACT_STATUS_WORKER_PANIC` when the
reason IS `domain53_war_worker_panic`. Writing a background cause there would
delete the perf blocks from a frame that succeeded and, for an unwound worker,
would stamp WORKER_PANIC onto a frame that did not fail. There is one clean
answer and it is implemented: a separate `background_worker_note` field, filled
from `tracker.war_validation_failure_reason` on every return, emitted as
`domain53_background_worker_note` (presence) plus the reason name, and touching
neither predicate, `domain53_partial_cause_mask`, nor the status flags. Both
containments are pinned by tests.

## 9. Coverage boundary after this lane

§4's boundary is narrowed, not removed. `domain53_sweep_stage` now holds the
reader's whole snapshot-driven stage — war-worker reap/spawn, the
generation-gated discovery rebuild, and the consume decision including WHICH
reason a cold frame names — as a pure function of the published snapshot plus
the tracker, so re-hardcoding `domain53_war_addrs_cold` at the consume site is
now caught (`d53_sweep_stage_names_the_worker_failure_not_a_hardcoded_cold`).

STILL not covered: `read_compact_diplomacy_intel_operatives_live` itself — its
needle resolution, its assembly of `Domain53Diag` from the tracker, and its
call to `domain53_source_unavailable_read`. Deleting the reader's call to
`domain53_sweep_stage`, or dropping `background_worker_note` /`stale_frames`
from the reader's `diag_ctx`, would still leave every test green. Closing that
needs the memory impl and `CollectorState` injected into the reader as well,
which is a materially larger refactor than the snapshot seam and was not
attempted here.

## 10. Review round 2 — five corrections (Codex 019fbbe3)

The first round of this lane's work was wrong or incomplete in five places.

1. **The shared service's health only rode d53's record.**
   `heap_sweep_diagnostic_units()` is extended into the diagnostic units of the
   d53 perf record inside `build_compact_extraction_gate_records`. Domains are
   independently schedulable, so a capture that ran d60 (or d43, or d47) but
   not d53 lost the spawn-failure evidence entirely — which defeats the point
   of §6. `capwar60_diagnostic_units` now seeds its list from
   `heap_sweep_diagnostic_units()`, so d60 carries it too.
   RESIDUAL, not closed: the four other sweep consumers (d43 market-trade, d47,
   the delayed-event domain, naval 49/50) still carry nothing. Closing it needs
   the extension hoisted out of the per-domain branches in
   `build_compact_extraction_gate_records`, or a domain-independent sweep-health
   record; that function belongs to the producer-migration lane, so this lane
   left it alone.

2. **The snapshot reset skipped a poisoned lock.** `heap_sweep_reset_published_
   snapshot` used `if let Ok(mut writer) = lock.write()`, so a poisoned lock
   silently left the previous epoch's WARM snapshot published — while the read
   side (`heap_sweep_snapshot`) recovers from poison and would therefore serve
   it as current. That is the one direction the reset must never fail in, and
   it falsified the §7 invariant. `heap_sweep_reset_snapshot_cell` now recovers
   the poison, and takes the lock as a parameter so the case is testable
   without poisoning the process-global cell.

3. **§7's conclusion was too broad.** The cold-window argument is correct and
   still stands: d53 fails closed for every frame of a dead epoch. But it says
   nothing about the frame AFTER the new epoch's first warm pass, where d53
   rebuilds discovery at the new generation and then consumes the war-address
   set validated under the DEAD epoch — on a CLEAN frame.

   The lane's original reasoning was that this is the same cross-generation
   retention d53 performs at every ordinary generation change. The distinction
   it missed is BOUNDEDNESS. Inside one epoch the retained set's age is bounded
   by the sweep cadence, which is the domain's declared staleness contract.
   Across an epoch death the outage lasts as long as thread creation keeps
   failing — bounded by nothing — and the resulting clean record silently omits
   every war created in between. Re-reading the cached addresses proves the
   rows it DOES carry are live; it cannot see a war that is not in the set.

   So d53 now has an epoch guard after all, in the narrow place that needs one:
   the tracker records `war_validated_epoch` (and the in-flight handle carries
   its spawn epoch, as d60's does), and the consume path drops the set and
   returns `domain53_war_addrs_sweep_epoch_changed` on mismatch. The
   operative-side rebuild continues to cover the cold window as described in
   §7; cross-GENERATION retention inside one epoch is unchanged and is pinned
   by the same test.

4. **`sweep_age_ms` was silently inflated.** §9 claimed moving the discovery
   rebuild ahead of the diag context was observationally neutral because the
   tracker fields are disjoint. They are — but `sweep_age_ms` was a fresh
   `Instant::now()` read, so on every rebuild frame it would have reported an
   age larger by the rebuild wall. It is now sampled inside
   `domain53_sweep_stage` at the original point (immediately after the
   war-validation reap) and returned.

5. **The reader's stamps were pinned by nothing.** The round-1 tests
   constructed `Domain53Diag` values directly, so dropping
   `background_worker_note` or `stale_frames` from the reader's context
   assembly stayed green; the same held for d60's `stale_frames` argument. The
   assembly is now `domain53_diag_context`, the reader's only statement for it,
   driven by a test; and `capwar60_source_unavailable_read` takes the TRACKER
   instead of a `u64` count, so a call site that passes 0 is unconstructible.

   The §9 boundary otherwise stands: deleting the reader's CALL to
   `domain53_sweep_stage` or to `domain53_diag_context` would still leave every
   test green.
