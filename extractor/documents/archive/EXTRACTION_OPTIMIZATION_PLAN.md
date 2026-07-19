# Extraction Optimization Reference — measured frame-time and replay-size evidence

**PURPOSE:** preserve reproducible performance/size measurements, proven nulls, and optimization
levers for the extractor. This is not a completion ledger or current execution queue. Current
project state, residuals, and the production gate live only in
[`MASTER_EXTRACTION_CONTRACT.md`](MASTER_EXTRACTION_CONTRACT.md); per-domain status lives only in
[`../native/domain_registry.json`](../native/domain_registry.json). Dated measurements below are
historical corpus evidence, not claims about the 2026-07-14 head.

## Optimization constraints

- Preserve the master contract's lossless hourly-or-finer production gate; an optimization must
  prove decoded/fold equivalence and must not mask failure/partial signaling.
- Optimize total CPU work, not thread count alone, so the extractor remains usable on weaker PCs.
  The historical target was <20 ms warm average on the dev rig with a 4–6-core companion run.

## Measured baseline (historical 2026-07-08/09 corpus)

- Warm frames, speed 2: ~46–62 ms shallow state, **108–120 ms deep-war**
  (`live_perf_20260708/patched_navcache_speed2_60s`: avg 108.2 ms, p95 141 ms, 3.17 M guarded
  reads/frame, 99.1 % region-cache hit).
- Reads are ~90 % of the frame; encode/append ≈ 4.7 ms. Concurrency is a proven NULL
  (rc 0→24 sweep flat,
  [`P2B_LEVERA_RESULTS.md`](../native/runs/perf_rearch_20260706/P2B_LEVERA_RESULTS.md)) — the
  frame was `sum(work)/cores` on that corpus.
- Stream volume: **6.57 MB/game-hour** busy mid-war
  (`live_perf_20260709/monitored_finalstate_speed2_20s`, 216.7 MB / 33 h). Full-game
  (~105,000 game-hours) projection: **~690 GB raw stream / ~300 GB shipped container**.
- The container ships **two full representations of every run**: raw compact stream (28 %,
  renderer never reads it) + per-hour folded JSON snapshots (72 %, the only thing
  `replay_core/source.rs` consumes). Container codec is deflate-6 (zip), measured 8.5×;
  lzma-6 measured **42.9×** on the same stream → ~5× codec headroom.

### Historical convergence result

The same five domains dominated both time and bytes on that corpus:

| Domain | id | deep read ms (avg) | share of stream bytes | delta in this corpus? |
|---|---|---|---|---|
| diplomacy_intel_operatives_live | 53 | 65.4 | 47.5 % | full-replace per clean frame |
| construction_buildings_state_damage_live | 48 | 36.8 | 4.5 % | NO — full snapshot hourly |
| supply_logistics_attrition_live | 44 | 22.8 | 16.1 % | NO — full snapshot hourly |
| projects_live | 41 | 20.7 | 16.7 % | NO — full snapshot hourly |
| scripted_state_flags_variables_live | 51 | 18.9 (FROZEN lane) | 8.8 % | NO — full snapshot hourly |

Together: ~93.6 % of stream bytes and the top of the time table. One workstream per lane
(bulk-read collapse + delta emission) attacks both targets simultaneously. Delta-enabled
lanes (map_control 5.07 M observed → 15 k emitted; map_detail, divisions, weather,
division_equipment) prove both the machinery and the changed-row fractions (0.3–0.7 %).

Shared mechanism of the expensive lanes: walk a large object array with one tiny guarded
read per field (construction: 127,728 reads for 354 KB ≈ 2.8 B/read). Counter-proof that
bulk reads work: `weather_live` reads 5.26 MB in **5 reads**, costs ~1.5 ms for 13 k rows.

## Measurement protocol

The master contract owns production acceptance. For optimization experiments, the durable
measurement rules were:

- **G1 lossless:** A/B/A shadow protocol — new path vs old path fold to byte-identical
  per-hour snapshots (or decoded-row equality) on the same live game, same-state bracketed
  (`u32[gs+0x468]` protocol). No lever ships without this.
- **G2 speed:** warm average frame time at speed 5 on a deep-war save, with the master's current
  hour-accounting gate applied.
- **G3 weaker-PC check:** same run with collector+game affinity-limited to 6 cores; frame must
  still meet the hourly gate with 0 skips.
- **G4 size:** measured bytes/game-hour on the same capture, projected to 105 k game-hours.
- Only live paired A/B counts (~±13 % single-capture noise); no cross-run comparisons at
  different state depths.
- **G5 bounded memory & post-processing:** recorder host peak private bytes should stay under
  ~2 GB regardless of session length, and replay finalization must complete in time
  proportional to a small constant × capture time (streaming, not batch). Evidence for the
  historical defect: 2026-07-09 ShortSpeed5 run
  (`production_speed5_ShortSpeed5_20260709_124210`) —
  collector_record.exe reached **27 GB working set / 34 GB private** and post-processing ran
  >15 min after a 300 s capture (hoi4.exe itself: 4.8 GB). User verdict: absurd, major
  problem.

## Historical speed-5 failure baseline (2026-07-09)

- `runs/production_speed5_ShortSpeed5_20260709_124210` (live mid-war, speed 5, 300 s,
   then-current full launcher shape) was a hard failure:
   - frame avg 69.9 ms, p95 82.4 ms, max 130.1 ms (sum of per-domain avgs 176.9 ms,
     parallel lanes); game advanced ~21 game-h/s.
   - **3,897 of 6,346 game hours SKIPPED (61 % data loss)** — the status quo already
      violated the zero-data-loss requirement at speed 5, establishing performance as a
      correctness concern for that run.
   - raw stream **28.15 GB / 300 s** (~11.5 MB per captured hour → ~1.2 TB full game).
     d53 emitted 855.9 M "changed" rows in 2,450 frames (~349 k/frame) — its delta path is
     effectively full-replace.
   - host RAM 27–34 GB + post-processing killed manually after ~55 min, still unfinished
     (G5 evidence). The partial fold had already written **~65 GB of per-hour snapshot
     JSONs** for ~2,449 captured hours (~27 MB/hour) on top of the 28 GB stream — the
      snapshot representation is ~2.3× the stream at deep-war. Partial snapshots were deleted
      2026-07-09; the 28 GB stream was retained as the bounded-memory scale-test input.
   - **stream not decodable**: 4 parse errors, `focus depth row N has no token context`
     (focus_research_decisions_depth_live encoder emits rows the host decoder rejects) —
      an independent gate failure in that historical run.
   - top read costs this save: d53 29.4 ms avg / 36.0 p95; map_detail 15.2; supply 14.2;
     divisions 13.4; division_equipment 13.0.

## Historical debug-tax finding (2026-07-09)

The 2026-07-09 audit found unconditional `DOMAIN_PERF` emission, production-enabled per-read RC
counters, and a divergent JSON-stream path. The durable design rule is to keep production and
debug on the same compact schemas/readers/fold while varying diagnostics only. Production must
retain hour accounting and fail-closed source/partial signaling; only instrumentation cost may be
removed.

## Historical bounded-memory streaming fold/pack result (2026-07-09)

Merge `f8cc48d6f` implemented streaming capture→disk with rolling SHA-256, mmap manifest gate,
file-backed incremental fold (hour spilled + released at FRAME_END), streaming zip pack with
per-chunk compression, streaming re-read verification, G5 telemetry (`host_finalize.*`) in
every manifest. Proof: streaming fold chain-hash == batch fold_check chain-hash on the real
216 MB stream (`be3d4994…`, independently reproduced by the orchestrator); peak private
1.13 GB and FLAT across 4× stream size (867 MB → +3 MB). Hardening round (Codex review →
Codex fix → re-review → orchestrator direct fixes): shared 2 GiB writer/reader entry cap
enforced mid-stream; zip32 checked casts + entry-count preflight (cap 65,534 — 0xFFFF is the
zip64 sentinel); unexpected pipe read errors now hard-fail instead of publishing a prefix
stream; verifier checks CRC-32/flags/local headers/data descriptors; fail-closed manifest
rewrite when the pack fails (no stale replay advertised). At this measurement point, the verifier
accepted only the writer's signed-descriptor form; the later container result below records the
zip64 implementation.


## Historical bulk-read and delta results (2026-07-09)

These are implementation/measurement records, not current domain status. Read-side collapse was
proven for d48, d44, d41, and d53. The d53 exact-offset reader removed an unused 832-byte ×
104-qword blob scan: the scan yielded zero emitted facts while the real flags came from the
  `rel+0xD0` vector and the subject-pointer `+0x288` pattern, eliminating about 53 MB/frame
  of memcpy and 6.7 M lookups. A later live shadow run reported zero relation mismatches.

The same 2026-07-09 change added baseline+delta+tombstone emission for d41/d44/d48/d51/d53
with fold-equivalence tests;
trackers did not advance on failed/partial frames. Exact current semantics belong to code and the
master gate, not this historical summary.

The residual-delta study measured d44/d51 on the deep-war corpus
(`runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin`, 12.9 GB;
4×300 MiB windows @ 0/35/70/~100 % → 28,964 records, 1.225 GB sampled). Per-domain byte
shares of the sampled stream: d53 85.3 %, d48 6.1 %, d20 3.6 %, **d44 1.16 %**, everything
else <0.5 %; **d51 total 0.05 %**.

- The five constant d44 badges cost **200 B/game-hour** (0.37% of d44, 0.0043% of the stream);
  delta-once emission removed the repeated bytes, with baseline coverage and fold identity gated.
- The d51 opaque badges cost about **165 B/game-hour** (0.0035% of the stream); zstd reduced this
  constant redundancy to about 1 B/game-hour, so a three-layer tombstone/fold rewrite was measured
  not worth the risk. Delayed-event rows would decrement `due_remaining_hours` every hour and
  re-emit even under delta; this corpus contained zero such rows (a corpus fact, not closure proof).

## Historical offline read-collapse sweep (2026-07-09)

Beyond the Big Five, bulk-read collapse (byte-identical contract, all-window-or-all-fallback,
equivalence + refused-window fixtures per lane) was demonstrated for every deep-war lane above
about 13 ms:
d51/d52/d56 frozen lanes (627a25f87/60c035914/586666f09), d23 map_detail + d26
division_equipment (35790f468/30d62ed9d), d28 oob_command + d29 ships (376c9e50f/14e84996c),
politics_live (921c7c9fa). `stream_fold` also gained `--discard-snapshots` (688ca2840) so
full-stream soaks fit disk.

## Historical container/codec result (2026-07-09)

The historical container implementation used zstd method-93 (level 13, window_log 27, LDM) plus
full zip64 in the streaming writer
and both readers (`222fae43f` + `f2631ca70`; controls included streamed file-backed entries,
reader allocation caps, EOCD-at-EOF validation, a checked-in pre-222 legacy fixture, and a
configurable `CompressionProfile`).
**Measured on the real 28 GB deep-war stream (2 GiB slice): zstd 163.2x @ 483.5 MiB/s vs
deflate-6 9.37x @ 52.6 MiB/s.** Legacy deflate containers remained readable.

## Historical live-validation result (2026-07-09 night)

Run: runs/live_validation_20260709_221706 (+ probe in live_validation_20260709_215202).
- **d53 shadow evidence:** `domain53_relation_shadow_mismatches == 0` on every frame with
  both paths live, validating the exact-offset relation reader on that build.
- **Light save probe** (restarted game, ~13.5 h/s): frame avg 36.4 ms, 1/162 hours skipped
  (cold frame), ~0.69 MB/hour (~17x smaller than baseline), host peak 604 MB.
- **Deep late-war save** (~6 h/s, the game 8.2 GB): frame avg 151.5 ms p95 263.6, 370/1,790
  hours skipped (20.7%), 12.9 GB/300 s raw (~9.1 MB/hour). NOT a paired A/B vs the morning
  baseline (different save depth — 21 h/s vs 6 h/s); it maps the remaining gap on heavy
  worlds. This was initially interpreted as proof that lossless change-driven capture was
  necessary; the skip-law and 1.2 ms-floor corrections below supersede that conclusion.
- Delta effectiveness on deep war: d51 0.3% changed, d41 8.3%, d44 10.0% (all working);
  **d48 79.2%**, traced to the tolerated-partial path described below. The contemporaneous
  attribution of **d53 ~full-churn** to genuine hourly opinion drift was later refuted by the
  d53 study below: tolerated-partial frames had disabled delta.

### Historical d48 tolerated-partial churn result

Root cause (CONFIRMED from code, not a stall): `COMPACT_STATUS_PARTIAL_DATA` is in
`COMPACT_STATUS_FAILURE_MASK` (compact_contract/src/lib.rs:186), so a tolerated-partial d48
frame (gate.ok, no hard failure, PARTIAL_DATA set) hit the early return in
`compact_domain48_apply_emit_mode` (collector/src/lib.rs ~21038) and emitted its **entire read
row set** — delta was bypassed on every partial hour. d48 was in the run's tolerated-partial
set, so ~every hour re-emitted ~its full rows (the 79.2%). NOT a stale-baseline diff; the delta
baseline was correct — it was simply never consulted on partials.
Manifest evidence (live_validation_20260709_221706): d48 `expected_partial_tolerances_applied`
`frames_affected = 1421` — d48 was tolerated-partial on essentially EVERY captured hour, and the
tolerance reason confirms the partial reads are accurate-but-incomplete ("valid
construction/building/resource rows retained… omitted siblings preserved"), i.e. the retained
rows are unchanged hour-to-hour — the ideal case for the projection-diff reduction below.
The fix added a `partial_projection` accumulator: tolerated-partial frames emit changed observed
rows only, never tombstone an unread omission, and leave the clean delta baseline pinned. Tests
covered unchanged/changed/reverted rows, deletion behavior, projection reset, and fold identity
against both full emission and the legacy partial stream. On the synthetic fixture, an unchanged
partial hour fell from 3 rows to **0**; a real change emitted exactly one row.

## Historical performance-corpus record

At the 2026-07-10 cleanup point, the deleted 2026-07-08/09 streams were represented by 54 retained
manifests. Neither those manifests nor the corpus paths cited in this section are present in this
worktree; treat them as historical external-artifact identifiers. The recorded morning numbers were
69.9 ms average / 82.4 p95 / 3,897 skipped / 28.15 GB on a lighter world and are not directly
comparable to the deep-war run.

The stream used by the studies below was:
`runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin` — deep-war,
~12 GB, 1421 hours. Reference fold chain hash over its first 600 MiB:
`bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7` (198 hours, 22,059 records),
reproduced identically by sequential and 4-worker parallel folds. No in-worktree copy or backup was
found during the 2026-07-14 audit.

## ⚠ CORRECTION 2026-07-10 (b) — skip law re-measured; "we cause the bursts" REFUTED

Full study:
[`archive/lane_reports/burst-correlation.md`](archive/lane_reports/burst-correlation.md).
Header-only scan of the deep-war stream, using
`elapsed_wall_ms` (frame_end payload `[0:8]`; `frame_time_us` is `[8:16]`, `collector/src/lib.rs:41189`)
so the game's pace is measured **independently** of our sweep cost.

- **We do NOT cause the bursts.** `corr(prev frame ms, interval of next cycle) = +0.4364` — after a
  *heavy* frame the game runs *slower*. The pattern is **common cause**: world workload drives both
  our sweep cost and the game's per-hour cost. Skips cluster where the game is **fast**
  (mean interval 101.1 ms on skipped cycles vs 211.0 ms on clean ones), not where we are slow.
- **The old "min inter-hour interval 28.2 ms" was a circular estimator** (`min(frame_time/gap)`, a
  lower bound). Measured from wall clock: **min 33.5 ms**, p1 44.3, p5 71.0, p50 176.0, mean 186.0.
  Our sweep is **75.5%** of the mean cycle, so cutting the sweep shrinks the cycle ~1:1.
- **The old "zero-skip needs ≤25 ms, unreachable" is WRONG.** Budget curve (game pace held fixed):
  **D ≤ 28 ms → 0 skips**; D = 40 → 0.1%; D = 60 → 1.1%; D = 100 → 7.1%; D = 151 → 19.5% (model
  predicts 344 vs **370 actual** at the observed D, 7% under). Target is a **~30–40 ms cycle**, a
  **~3.8× cut** from 151 ms — and d53 measured ~3× above its pure memory-latency floor. Plausibly
  reachable by cheaper acquisition alone, with **no change-driven read scheme**.
- **Confound later measured:** the curve assumed the game's pace was exogenous, while the
  `+0.4364` correlation could not separate common cause from causation. The later fixed-speed
  subset experiment in [`PRODUCTION_ENDGAME_PLAN.md`](PRODUCTION_ENDGAME_PLAN.md) measured
  about 6–7% pace cost from roughly 90–100 ms of extraction work on its corpus. Therefore
  **D≈28 ⇒ zero skips remains an upper bound, not a prediction**.
- **Acquire/process split — ceiling already bounded.** d53's encode+decode-gate is **7.9 ms** vs a
  **49.3 ms** read: processing ≈14% of that domain's wall, so deferring *encode* buys little. The
  split pays only if **materialisation** is deferrable, which needs a raw-byte copy at the hour
  boundary. Instrument **three** phases before building it: (1) guarded reads touching game memory
  (not deferrable), (2) materialise/decode (deferrable only behind a raw copy), (3) encode/emit
  (deferrable, known-small).

> **Method warning.** Two earlier passes of this study misread `elapsed_wall_ms` as `frame_time_us`
> and produced confident nonsense (`corr = +0.9302`, `autocorr(ft,1) = 1.0000`). Validate any
> reconstructed series against the manifest (frame mean 151.5 / p95 263.6 / max 477.2) **before**
> interpreting it.

Retained d53 safety caveat: the historical fold re-seeded hour overlays from the mirror baseline
on every delta frame. If sampling ever permits multiple d53 records in one game-hour, revalidate
that it does not keep only the last record where full emission would union them. The invariant is
fold identity with the defined partial semantics, not a blanket "never tombstone from partial."

## Sampling-loss semantics (clarified 2026-07-10)

**An async sampler of a sim it cannot pause cannot deliver literal zero data loss.** Two facts:

1. **Intra-hour transients are unobservable to any hourly sampler.** A value that changes and
   reverts within one game-hour is invisible no matter how fast the sweep is. Hourly cadence is
   the requirement; hourly cadence is also the ceiling on what can be seen.
2. **A skipped/coalesced hour IS real loss of that hour's world state.** The historical deep-war
   run lost 370 of 1790 hours (20.7%). A latch makes loss accounted rather than silent; it does
   not recover the missing state.

The resulting accounting invariant was:

> **ZERO-SILENT-GAP ACCOUNTING + BOUNDED SKEW.** Every game-hour is observed and accounted as
> either a capture attempt (confirmed-complete or recorded-failed) or a **reason-tagged**
> coalesce (superseded / overflow / flush / backward / anomalous). Invariant:
> `observed == attempts + coalesced + in_flight`, with `in_flight == 0` after flush. Read skew
> exists and is bounded, not monotonically improved.

**Capturing every hour remains the goal — and it is a SWEEP-SPEED problem, not a latch problem.**
Under sustained overrun only ~`RUN_MS / sweep_ms` hours can be captured; the rest are physically
unrecoverable. The latch buys correct accounting and absorbs *momentary* short intervals. Getting
hours captured rather than coalesced still requires a sufficiently fast sweep.

**Crucially, silent loss is prevented by DELTA accounting, not by the poll rate.**
`HourLatch::observe` accounts every hour in `last+1..=raw`, so a multi-hour jump between polls is
detected and coalesced-with-reason. 28.2 ms is the minimum *observed* interval in one capture, not
a proven floor — correctness must never depend on it, and does not.

## ⚠ CORRECTION 2026-07-10 — the Phase 4 "1.2 ms floor" is UNSUPPORTED

The archived
[`phase4-mirror-mapdetail.md`](archive/lane_reports/phase4-mirror-mapdetail.md) study refuted the
premise that `map_control`'s 40x advantage comes from a dense-mirror/change-driven read.
Verified independently against the source:

- `map_control` reads **one scalar at a fixed offset inside an already-resolved object**
  (`collector/src/lib.rs:33545`) and walks **every** entity **every** frame. It is *not*
  change-driven at read time — its 5.07M→15k reduction is ordinary **delta emission**, which
  every domain already has.
- `map_detail` chases `pdx::vector<CBuilding*>` and reads `CBuilding.level` (+0x40) from
  individually heap-allocated 0x1F0-byte objects. **No contiguous building-level array exists**
  in the engine. It is *already* change-driven collector-side. Part of its 49.2 ms is also an
  impure measure: a `Mutex<BTreeMap>` cache that clones a `Vec` per lookup
  (`collector/src/lib.rs:6135`).

**The 40x gap is DATA SHAPE, not change-driven-vs-full-read.** Therefore, in the lever table
below, the projection `proj = max(1.2 ms, changed_frac × read)` — and every row derived from it,
including lever **1b (map_detail 49.2→1.2 ms)** — is **unsupported** for any domain whose fields
live in scattered per-object heap allocations. The 1.2 ms floor is reachable only where the
needed field is a scalar inside an object the topology already resolved. **Re-derive the floor
per domain against its actual data shape before quoting a saving.**

What survives unchanged: **no engine dirty signal exists**; any change-driven scheme must be
*complete by construction*; and 77% of read-work still reads provably-unchanged rows (that is a
statement about what is read, not about how cheaply it could be read).

The later paused-game `d23_locality_probe.py` test settled the region-coalesced bulk-read
question for d23 as a null; see **Known nulls** below. The data-shape warning still applies to
other scattered-object domains unless their locality is measured independently.

## Historical deep-war corpus profile (2026-07-09 night)

Source: `runs/live_validation_20260709_221706/stage2_shortspeed5` — manifest per-domain
`read_time_us` + a full header-only scan of the 12.0 GB `collector_stream.bin` (1421 frame_end
records for per-frame `frame_time_us`, plus per-domain DOMAIN_DATA payload bytes from the H4CR
headers; scan script in lane scratchpad, read-in-place). Frame avg 151.5 ms, p95 263.6, max
477.2; 1421 hours captured / 370 skipped of 1790 advanced (20.7%); 9.068 MB/hour (matches the
9.08 reported to 3 s.f.). Numbers below are that run only.

### Measured per-domain frame time

The sum of per-domain avg `read_time_us` is **693.6 ms**, but the frame avg is **151.5 ms**
(`reader_concurrency=12`). So the frame is **critical-path bound at ~4.58× effective
parallelism**, NOT the sum — cutting a domain only helps the frame if it sits on the slow lane.
Top read-cost domains (avg ms / p95 ms / % of the 693.6 ms read-sum):

| domain | id | avg ms | p95 ms | % read-sum | changed/present |
|---|---:|---:|---:|---:|---:|
| supply_logistics_attrition_live | 44 | 60.0 | 123.6 | 8.6% | 10.0% |
| oob_command | 28 | 58.6 | 93.8 | 8.4% | **0.7%** |
| ships | 29 | 57.9 | 94.3 | 8.3% | 8.7% |
| diplomacy_intel_operatives_live | 53 | 49.3 | 84.0 | 7.1% | ~100%+ churn |
| map_detail | 23 | 49.2 | 99.6 | 7.1% | **0.1%** |
| division_equipment | 26 | 41.9 | 91.8 | 6.0% | **2.2%** |
| divisions | 20 | 41.3 | 91.0 | 5.9% | 58.6% |
| politics_live | 14 | 37.9 | 82.6 | 5.5% | **0.6%** |
| naval_combat_lifecycle+members | 49/50 | 34.1 (shared walk) | 74.9 | 4.9%×2 | 4.4% / 19.3% |
| occupation_resistance_compliance | 52 | 25.1 | 53.8 | 3.6% | **1.3%** |
| naval_missions_status_live | 56 | 24.7 | 52.2 | 3.6% | **0.8%** |
| projects_live | 41 | 24.4 | 53.0 | 3.5% | 8.3% |
| scripted_state_flags_variables | 51 | 23.1 | 52.5 | 3.3% | **0.3%** |
| construction (d48) | 48 | 22.2 | 50.5 | 3.2% | **79.2%** |

### Measured byte distribution (9.068 MB/hour, post-delta)

Header-summed DOMAIN_DATA bytes/hour: **d53 diplomacy_intel_operatives = 8.372 MB = 92.3%** of the
entire stream. Everything else is noise: d48 construction 288 KB (3.2%), divisions 169 KB (1.9%),
supply 52 KB (0.6%), capitulation 35 KB (0.4%). Delta cross-check (changed_rows/row_count,
cumulative) reproduces the reported figures: d51 0.3%, d41 8.3%, d44 10.0%, **d48 79.2%**, and d53
churns >100% (242 M opinion rows re-emit hourly). On this corpus, d53 dominated bytes; the later
d48 partial-projection fix addressed the measured d48 artifact. zstd-93 was the intended absorber
for repeated d53 payloads, with the d53 study below testing finer delta options.

### Surviving lever guidance

The modeled 1.2 ms floor and every saving projected from it were removed after the data-shape
correction above. Durable guidance: profile the actual critical lane; prove address locality/data
shape before proposing bulk or coalesced reads; treat d53 acquisition separately; and require live
paired A/B plus fold identity for every claimed win. The lane-wall instrumentation and d53 study
below provide the measured starting points.

### Historical post-capture fold result (2026-07-10; 5.4x measured)

Profile first (FOLD_PROFILE=1 env-gated phase timers, deep-war 512 MiB prefix, 189 hours,
release, this rig): fold wall 173.1 s = **build_snapshot 65%** (75.2 s), apply_domain_data
10.5% (12.2 s), serde_json serialize 10% (11.5 s), snapshot write 3.8% (4.4 s). Input
512 MiB expands to 5.13 GB of per-hour snapshot JSON (14x amplification, 27–55 MB/hour) —
serialization/IO were never the wall; per-hour `Value`-tree projection is.

The measured configuration used these levers (all G1-gated byte-identical, all
orchestration-layer — zero domain-fold body edits):
1. **Background emit pipeline**: FRAME_END hands the built snapshot `Value` to a bounded
   (depth 2) worker that serializes straight into the spill file through a SHA-256-tee
   writer (serialize+write+hash off the critical path, no intermediate Vec, no re-read for
   the report hash; `serde_json::to_writer` == `to_vec` bytes). 173.1 s → 128.6 s.
2. **Parallel strided fold** (`fold_compact_stream_file_parallel`): hours are NOT
   independent fold units (delta accumulators carry across hours), so N workers each fold
   the WHOLE stream (apply is ~10% of cost) and worker k builds+emits only hours with
   emission ordinal ≡ k (mod N). `build_snapshot` is `&self`-pure, so skipping it on
   non-owned hours cannot perturb state — union output is byte-identical by construction;
   worker accounting divergence fails loudly. Recorder-host finalize + stream_fold
   `--workers N` (default = cores/2 clamp; 8-core knee is 4 — memory-bandwidth-bound, 8
   workers is SLOWER).
3. **mimalloc global allocator** (both bins): the fold is allocation-bound (multi-GB Value
   trees built/dropped per hour); Windows heap serializes badly across workers. G1-neutral.

Measured on the SAME deep-war 512 MiB prefix, same rig, back-to-back: 3.07 MB/s baseline →
4.14 (emit pipeline) → 11.15 (4 workers) → **16.59 MB/s (4 workers + mimalloc) = 5.4x**,
peak WS 3.09 GB (vs 1.05 GB baseline). 2 GiB prefix, 4 workers: 23.45 MB/s, 349 hours,
peak WS 8.52 GB. G1 evidence: chain hash + all 189 per-hour SHA-256 byte-identical to the
pre-change baseline fold on every configuration; determinism gate: two identical-input
parallel runs → identical chain (both 1w-pipelined and 8w). 12.9 GB deep-war projection:
~40 min → **~9–13 min**.

A separate historical commit added `--skip-host-fold` — it skips the host fold
entirely (raw compact stream is the canonical artifact; replay_decoder D1 gives consumers
per-hour access), packs an empty snapshots array, and marks the manifest + container index
`host_fold_skipped` explicitly. Default behavior unchanged.

## Historical change-driven RE evidence (2026-07-10)

Full archived evidence, addresses, and decompile snippets:
[`archive/PHASE4_DIRTY_SIGNAL_SCOUT.md`](archive/PHASE4_DIRTY_SIGNAL_SCOUT.md).

- **No engine-native per-object dirty/version signal exists.** Confirmed by static RE of
  `hoi4.exe` (Ghidra, image base `0x140000000`): the relation opinion writer at `0xd09d50`
  bumps no version; `CUpdateable` (`updateableinterface.cpp`) is a pure-abstract base with no
  dirty bit; the only `dirty` strings in the binary are UI tree relayout. The MP desync
  **checksum is a monolithic O(entire-world) serialize walk** (`0x1f2a20` size 8376 / per-
  category mgr `0xd96cb0` size 12878), computed on demand, a single hash — **as expensive as
  the full read and unable to say which row changed. Do NOT build Phase 4 on the checksum.**
- Cheap complete reads must be proved per domain from its real data shape. The later
  map-detail review above refuted using `map_control` as universal dense-mirror evidence;
  scattered objects require locality measurement, not a projected 1.2 ms floor.
- **Zero-skip latch (orthogonal to "what changed"):** main-thread hour-boundary hook site found
  `postHourlyUpdate`, reads the hour counter as `(gs[0x468]-43800000) % 24`, asserts
  `ThreadIsMainThread()`, fans out via TBB. A non-mutating atomic-store detour at
  `postHourlyUpdate` latches every hour (never missed) decoupled from the heavy decode; risk
  HIGH (main-thread sim patch → MP-checksum control gate required). See scout doc §Hour-latch.
- **Fallback if a heavy has no mirror:** OS `GetWriteWatch`/`PAGE_GUARD` — complete but 4 KiB
  page-granular and hostile to the game's shared-page heap layout; low selectivity, last resort.

## Historical per-frame lane-wall instrumentation result (2026-07-10)

WHY: deep-war analysis (2026-07-09 night) measured sum(per-domain avg `read_time_us`) =
693.6 ms vs frame avg 151.5 ms at reader_concurrency=12 (~4.58x effective parallelism). The
frame is CRITICAL-PATH bound, but nothing recorded which domains shared a reader lane or
which lane's wall bounded the frame, so every per-domain ms-saved claim carried a 1x–4.58x
attribution band. This lever removes that blindness. Instrumentation only — zero
capture-semantics change, all profiles.

The branch `lane/lane-wall-counter` implemented:
- `ReaderGate` permits now carry a lane identity (0..reader_concurrency; free-list
  semaphore, scheduling semantics unchanged). Every top-level fan-out worker records a
  `(domain tag, lane, start_us, end_us)` span (offsets from `worker_read_started`; span
  starts AFTER permit acquire so queue wait is not counted as work). Post-join serial reads
  (d55/d54/d45/d47/d18/d63..d69) record under the synthetic MAIN lane (65535). The
  country-bundle worker reports as synthetic tag 200 (compact domains 10..=17).
- Per frame the collector emits summable `lw_*` diagnostic units on the reserved probe
  domain 250 in EVERY profile (production included). Safety: old and new hosts exclude
  domain 250 from every coverage/perf-count gate, the fold ignores PERF records, fold_check
  counts them as `perf_records_ignored`. Units: `lw_frames`, `lw_worker_span_us`,
  `lw_crit_wall_us`, `lw_crit_busy_us`, per lane `lw_l{n}_wall_us`/`lw_l{n}_busy_us`
  (n < 32; overflow folds into `lw_lother_wall_us`), `lw_lmain_wall_us`, and
  `lw_d{id}_crit_us` for each domain on the critical-path lane (argmax lane wall,
  ties -> lowest lane).
- Debug sidecar (`collector_compact_phase_timing.jsonl`, `--sidecar-log`) additionally
  carries the full per-frame assignment:
  `"lane_wall":{"crit_lane":..,"crit_wall_us":..,"spans":[[tag,lane,start_us,end_us],...]}`.
- Manifest (production): `compact_stream.lane_wall` = `frames`, `crit_wall_us`
  (count/avg/p95/max), `crit_wall_us_total`, `crit_busy_us_total`, `main_wall_us_total`,
  `other_wall_us_total`, `per_lane.l{n}` (wall count/avg/p95/max + busy total),
  `per_domain_critical_path_us_total`, `per_domain_critical_path_share`.

OFFLINE FORMULAS (derivable from the manifest alone):
- avg critical-path wall/frame = `crit_wall_us_total / frames` (should closely track
  `frame_time_us` minus prewarm + MAIN-lane serial reads + encode/append).
- critical-path share of domain d = `per_domain_critical_path_us_total[d] / crit_wall_us_total`.
- critical-path ms/frame of domain d = `per_domain_critical_path_us_total[d] / frames / 1000`
  — the exact ms-saved upper bound for optimizing d (replaces the 1x–4.58x band).
- scheduler slack on the critical lane = `crit_wall_us_total - crit_busy_us_total`
  (idle gaps between assignments on the bounding lane).

WIRE FORMAT: unchanged — reuses the existing DOMAIN_PERF + PDU1 diagnostic-unit trailer and
the reserved probe domain 250 (no compact_contract change needed). Old streams decode
identically (host test `lane_wall_absent_in_old_streams_is_zero`); new streams decode on old
hosts because domain 250 was already skipped wholesale. fold_check hash on the legacy
fixture is byte-identical pre/post (gate evidence in the landing commit message).

COST: measured 4.81 us/frame in release (`lane_wall_micro_cost_measure`, 2000 frames, 48
spans/frame — the full added path: span recording + summary + probe-record encode); budget
was <100 us. Per worker it is 2 timestamps + 1 uncontended mutex push, outside every domain
read loop.

FIRST LIVE NUMBERS (2026-07-10 late session, deep-war advancing production runs — evidence
of record for Job 2's fork; mined via `compact_stream.lane_wall`):

| run | frames | crit lane avg / p95 | d53 crit share | d53 ms/frame | MAIN serial ms/frame | span ms/frame |
|---|---:|---:|---:|---:|---:|---:|
| latchsoak2 (600 s) | 3471 | 60.7 / 108.4 ms | 71.1% | 43.2 | 42.0 | 106.9 |
| ShortSpeed5 (600 s) | 4370 | 46.5 / 73.5 ms | 75.7% | 35.2 | 41.9 | 91.4 |

Historical readings:
- **d53 owned ~3/4 of the bounding concurrent lane** (next domains: country-bundle ~5-7%,
  d44 ~6%) in these two runs.
- The runs exposed a **~42 ms/frame serial main-thread wall** (`main_wall_us_total/frames`,
  stable across both runs): the post-join serial reads (d55/d54/d45/d47/d18/d63..d69 +
  sharded domains) extend every frame serially and are nearly as large as the whole
  concurrent phase. At the time, the candidate was to move post-join
  serial reads onto gated lanes (they were serialized for a reason — re-derive that reason
  before touching; if it is ordering, a dedicated ordered lane still overlaps them with d53's
  tail). This observation was unmeasured beyond the two runs and is not a current work-status claim.
- GOTCHA (cost the session an experiment rerun): on a PAUSED game only the 1 cold baseline
  frame runs the fan-out ⇒ `lane_wall.frames == 1` and `lw_d*` looks "empty". The earlier
  "empty lw_d walls" report was exactly this artifact. Always gate-bracket (hours advanced
  > 0) before reading lane_wall.

## Acceptance routing

The combined speed-5 proof is defined and tracked only in the master contract. Its performance
report should include frame average/p95/max, bytes/game-hour, projected full-game size,
fold-equivalence hash chain, and a core-constrained companion result.

## Durable optimization decisions (2026-07-09)

The stated end goal: hand the `.battleplan` to the replay layer and watch the entire game
play out in absolute detail; the replay layer is being built as a reverse-engineered game
engine that consumes the extracted data directly (read-only playback). Full architectural
free reign granted, bounded only by the zero-data-loss requirement.

- The compact typed stream is the canonical full-detail record for direct replay consumption.
- Frozen lanes (d51; d47/d52/d56 if needed) may be
  rewritten under a fold-identical gate (stream bytes may change; decoded state must not).
- Any change-driven optimization must have an RE-proven completeness mechanism plus
  soak-validated fold identity.

Non-negotiable consequence of the end goal: the production stream must always remain a
complete, self-sufficient full-detail record — no lever may ship that makes the stream
lossy relative to the recorded decoded-state contract.

## Known nulls — do not revisit

- **d23 map_detail COALESCED READER — NULL, buried 2026-07-11.** Valid PAUSED-game locality
  probe (pause-watcher captured it at 01:23 after gate-bracket-discarding advancing attempts;
  runs/pause_watcher_20260710.log): CBuilding objects NEVER merge at gap<=0x100 (0.0% call
  reduction — same-size slab objects are interleaved with other allocations, never adjacent),
  and every useful cap pays massive dead-copy amplification (gap 0x400: 96% fewer calls but
  7.7x bytes; 0x4000: 99.5% fewer, ~9x). Pre-registered decision gate required BOTH large call
  reduction AND low amplification — no cap satisfies it, and the read-count!=time law already
  showed the deleted calls are cheap MRU hits. Pattern stable across a dozen snapshots as the
  world grew 8.6k->9.7k buildings. The d23 time floor stays where phase4-mirror-mapdetail left
  it (pointer-chase data shape, no dense mirror).


- Reader-thread concurrency tuning (flat 54–62 ms across rc 0→24).
- "Fewer guarded reads" without a mechanism (cheap MRU-hit reads cost ~nothing; here the
  3.17 M-read volume IS expensive at ~34–320 ns/read, which is why bulk-collapse works).
- Read-primitive swaps (SEH + region cache already banked, 3.57×).
- JSON stream format as a debug profile (divergent dead path).

## Historical d53 per-field study (2026-07-09; status superseded 2026-07-14)

Study scope: (1) make d53 exact-offset relation reader the unconditional default and strip
per-frame scan cost; (2) per-field delta for relation rows; (3) row-level delta for the d53
full-emit residual subdomains. Measurement on the REAL deep-war stream re-scoped 2 and 3.

**Offset-reader evidence.** At the time, `offsets` was the default in the collector, host
(`recorder_host` default `Offsets`), and lane (`DOMAIN53_RELATION_READ_MODE` AtomicU8
default `Offsets`); `scan`/`shadow` stay compiled + runtime-selectable. Removed the last
per-frame scan cost from the default Offsets path: `read_relation_rows_impl` now allocates
the 832-byte legacy blob-scan buffer and computes the vtable min/max scan bounds ONLY under
the Scan pass (shadow referee). Offsets pays zero scan setup. Byte-identity preserved:
`relation_offsets_mode_rows_are_byte_identical_to_scan_mode` + shadow tests green (lane
113/113). **Shadow re-validation is REQUIRED after any hoi4.exe patch** (run
`d53_relation_read_mode:"shadow"`, require `domain53_relation_shadow_mismatches == 0` on
every frame) before trusting offsets on the new build — the scan path is retired from the
hot loop but retained as the re-validation oracle.

**Decisive measurement.** Tool
`diplomacy_intel_operatives_live` bin `d53_field_delta_measure` decoded the first 400 d53
DOMAIN_DATA frames of
`runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin` (400 distinct
game-hours, 254 non-empty, `d53_emit_mode=delta`, `d53_relation_read_mode=offsets`), read in
place via the authoritative `decode_snapshot_payload`:

- **The d53 delta path NEVER FIRES in deep war.** All 400 frames carry `PARTIAL_DATA` and
  ZERO carry `DELTA_DATA` — avg **5.92 MB/frame FULL re-emit every game-hour**. Mechanism:
  a single torn per-row read across ~245 k rows/frame sets `PARTIAL_DATA`; the collector
  then skips delta (`collector/src/lib.rs` ~18081 `if PARTIAL { return full }`) and the fold
  **rejects `DELTA|PARTIAL` outright** (`recorder_host/src/world_fold.rs:10530`
  `delta_partial_unsupported`). Deep-war frames are ~always partial, so delta is
  architecturally disabled there. (This is the true cause of the plan's line-85 "855.9 M
  changed rows / delta path effectively full-replace" — NOT genuine opinion churn.)
- **Row-to-row hourly change is negligible.** Of re-emitted rows, the value field changed
  hour-to-hour in: opinion **~0.0 %** (48.1 M rows), state_event value **~0.0 %** (49.0 M),
  country casualties 0.7 %, war casualties 0.2 %, operative experience 3.9 %, cryptology
  amount 3.6 %, agency/cluster/subject fields ~0 %. The prompt's premise ("opinion values
  drift hourly across most pairs") is **refuted by the data** — opinions are static in this
  settled deep-war state. If row-level delta FIRED, >99 % of the 5.9 MB/frame would vanish.

**Per-field delta conclusion.** Per-field delta applied only on delta frames in the measured
implementation, so it saved **0 bytes** on this corpus.
Even hypothetically (if delta fired): per-field would omit the unchanged non-key field bytes
= **33.6 % of changed-row bytes** (opinion 25 %, state_event 37.5 %, agency 83 %, operative
79 %, cluster/subject 50–67 %). But d53's dominant rows are key-dominated — OpinionRow is
key(8)+opinion(4) and StateEventRow's two value fields co-change — and changed rows are ~0 %
of the frame, so per-field's ABSOLUTE contribution to d53 volume is ~0. Implementing a
versioned per-field wire format across encoder/decoder/fold/collector would add per-frame
field-compare CPU (violating the weaker-PC CPU law) for ~0 production benefit and expand the
losslessness attack surface. The study did not implement it; it was marginal even after
partial-frame delta.

**Full-emit residual conclusion.**
Per-frame wire volume of the named residuals: `factions` **~408 B/frame** (changes only
2/254 frames), `operation_instances` **~77 B/frame** (3/254), `incoming_actions` ~0,
`intel_network_graphs` / `operation_anchors` / `intel_map` **EMPTY** (0 frames present).
Combined **<500 B/frame vs a 5.9 MB frame = 0.008 %**. Row-level delta would save ~99 % of
that (~480 B/frame) but only on delta frames, which never fire. Absolute saving is
negligible; the study did not implement it.

**Historical follow-up design (implementation status superseded 2026-07-14):** enable delta on
tolerated-partial frames. This was a semantics redesign, not a plumbing toggle:
`domain53_country_overlays_this_hour` is CLEARED at every hour reset (`world_fold.rs:1431`),
so in a deep-war hour (one partial frame) the snapshot then equaled that frame's observed set:
torn rows are silently DROPPED from the hour, and in-game deletions are conveyed only
implicitly by the full re-emit. A baseline-seeded delta variant must therefore define, and
prove, per-key semantics: (a) collector diffs observed rows vs the last non-partial
baseline (two-accumulator: the baseline does NOT advance on partial frames) and emits
`DELTA|PARTIAL` with changed/new observed rows; (b) POSITIVE-absence tombstones only — the
relation walk distinguishes slot==0 (positively absent → tombstone is safe) from a torn
read (unknown → NO tombstone; fold retains the baseline value, which is better than the old
drop-for-the-hour but is a deliberate semantics change to document); (c) fold accepts
`DELTA|PARTIAL` by seeding the hour from the delta baseline + upserting, WITHOUT advancing
the baseline (replaces the `delta_partial_unsupported` rejection at `world_fold.rs:10530`);
(d) automated gate: fold(delta stream) byte-identical to fold(full stream) under the SAME
new partial semantics, plus randomized-sequence round-trip property tests. Projected from
the measured stream: d53 **~5.9 MB/hour → tens of KB/hour (>99 % reduction)** at zero data
loss, and it also deletes most of the ~39 ms/frame payload-encode CPU (the
`seg_payload` residual) since only changed rows are encoded. This is a losslessness-
critical redesign whose losslessness proof must be fold identity over recorded/synthetic streams.
Check current code and the master contract before treating any part of this old proposal as open.
