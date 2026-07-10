# Extraction Optimization Plan — sub-20 ms frames + lean replays (2026-07-09)

Owner: extraction lane. Status: ACTIVE — phases 1/1b/2/3 landed, Phase 4 (change-driven
capture) in progress. See the dated status sections below for what is proven vs projected.

**Delegation (2026-07-10):** Codex CLI is the sole delegation engine and adversarial reviewer,
pinned `-m gpt-5.6-sol -c model_reasoning_effort=xhigh`. The earlier "Codex unavailable / do not
depend on it" note is DEAD.

**Lanes: read [`CODEX_LANE_BRIEF.md`](CODEX_LANE_BRIEF.md) first** — it carries the hard
requirements, the established facts, the proven NULLs, and the gates. **Lanes must not edit this
plan**; write [`lane_reports/<lane>.md`](lane_reports/) instead (concurrent edits here conflict).
The orchestrator folds lane reports into this document.

## Hard requirements (user, 2026-07-09 — binding)

1. **Zero data loss.** Every domain captured at hourly-or-finer cadence. Cadence thinning
   (domain every N>1 game-hours) is FORBIDDEN. A skipped re-read is legal only when the value
   is provably unchanged (lossless delta); heuristics must be validated fold-identical before
   shipping.
2. **Portability.** The extractor must run on weaker PCs than the dev rig. The deep-war frame
   is CPU-throughput-bound (work/cores), so levers must cut total CPU work per frame, not add
   threads. Target on the dev rig is <20 ms warm avg **with margin** (internal target ~12–15 ms);
   every acceptance run gets a core-constrained companion run (affinity-limited to emulate a
   4–6 core host).

## Measured baseline (evidence-of-record)

- Current production bundle: **55 compact domains**, all-native (no Python on the hot path).
- Warm frames, speed 2: ~46–62 ms shallow state, **108–120 ms deep-war**
  (`live_perf_20260708/patched_navcache_speed2_60s`: avg 108.2 ms, p95 141 ms, 3.17 M guarded
  reads/frame, 99.1 % region-cache hit). No clean speed-5 zero-skip run exists at head.
- Reads are ~90 % of the frame; encode/append ≈ 4.7 ms. Concurrency is a proven NULL
  (rc 0→24 sweep flat, `P2B_LEVERA_RESULTS.md`) — the frame is `sum(work)/cores`.
- Stream volume: **6.57 MB/game-hour** busy mid-war
  (`live_perf_20260709/monitored_finalstate_speed2_20s`, 216.7 MB / 33 h). Full-game
  (~105,000 game-hours) projection: **~690 GB raw stream / ~300 GB shipped container**.
- The container ships **two full representations of every run**: raw compact stream (28 %,
  renderer never reads it) + per-hour folded JSON snapshots (72 %, the only thing
  `replay_core/source.rs` consumes). Container codec is deflate-6 (zip), measured 8.5×;
  lzma-6 measured **42.9×** on the same stream → ~5× codec headroom.

### The convergence fact

The **same five domains dominate both problems**:

| Domain | id | deep read ms (avg) | share of stream bytes | delta today? |
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

## Acceptance gates (all phases)

- **G1 lossless:** A/B/A shadow protocol — new path vs old path fold to byte-identical
  per-hour snapshots (or decoded-row equality) on the same live game, same-state bracketed
  (`u32[gs+0x468]` protocol). No lever ships without this.
- **G2 speed:** warm avg frame at speed 5, zero skipped hours, deep-war save, dev rig.
  Phase targets below; final <20 ms (internal 12–15 ms).
- **G3 portability:** same run with collector+game affinity-limited to 6 cores; frame must
  still meet the hourly gate with 0 skips.
- **G4 size:** measured bytes/game-hour on the same capture, projected to 105 k game-hours.
- Only live paired A/B counts (~±13 % single-capture noise); no cross-run comparisons at
  different state depths.
- **G5 bounded memory & post-processing:** recorder host peak private bytes must stay under
  ~2 GB regardless of session length, and replay finalization must complete in time
  proportional to a small constant × capture time (streaming, not batch). Evidence for the
  defect: 2026-07-09 ShortSpeed5 run (`production_speed5_ShortSpeed5_20260709_124210`) —
  collector_record.exe reached **27 GB working set / 34 GB private** and post-processing ran
  >15 min after a 300 s capture (hoi4.exe itself: 4.8 GB). User verdict: absurd, major
  problem. A full-game capture cannot complete under the current batch fold/pack design.

## Phase 0 — Ground truth (small)

1. ~~One clean speed-5 run at current head~~ **DONE 2026-07-09** —
   `runs/production_speed5_ShortSpeed5_20260709_124210` (live mid-war, speed 5, 300 s,
   55 domains, full launcher production shape). **The honest baseline is a hard FAIL:**
   - frame avg 69.9 ms, p95 82.4 ms, max 130.1 ms (sum of per-domain avgs 176.9 ms,
     parallel lanes); game advanced ~21 game-h/s.
   - **3,897 of 6,346 game hours SKIPPED (61 % data loss)** — the status quo already
     violates the zero-data-loss requirement at speed 5. Phase 2 is a correctness fix.
   - raw stream **28.15 GB / 300 s** (~11.5 MB per captured hour → ~1.2 TB full game).
     d53 emitted 855.9 M "changed" rows in 2,450 frames (~349 k/frame) — its delta path is
     effectively full-replace.
   - host RAM 27–34 GB + post-processing killed manually after ~55 min, still unfinished
     (G5 evidence). The partial fold had already written **~65 GB of per-hour snapshot
     JSONs** for ~2,449 captured hours (~27 MB/hour) on top of the 28 GB stream — the
     snapshot representation is ~2.3× the stream at deep-war, confirming Phase 3's
     keyframe/binary/zstd priorities. Partial snapshots deleted 2026-07-09; the 28 GB
     stream is retained as the Phase 1b/G5 scale-test input.
   - **stream not decodable**: 4 parse errors, `focus depth row N has no token context`
     (focus_research_decisions_depth_live encoder emits rows the host decoder rejects) —
     BUG, gate-failing independently of skips. Fix in Phase 1b window.
   - top read costs this save: d53 29.4 ms avg / 36.0 p95; map_detail 15.2; supply 14.2;
     divisions 13.4; division_equipment 13.0.
2. 6-core portability run: DEFERRED until after Phase 2 (measuring a known failure adds
   nothing while frames cost ~70 ms and 61 % of hours already skip at full cores).
3. Fold-equivalence harness (G1): in progress (fold_check tool).

## Phase 1 — Strip debug taxes + `--profile` switch (fast, low risk)

Findings: DOMAIN_PERF records are emitted unconditionally (collector `lib.rs:13249` + ~30
sites) and byte-copied into every container; the production launcher passes
`--extraction-perf-gate`, turning per-read RC counters ON in production; the launcher also
omits `--lightweight-root-frame`; `--stream-format json` is a divergent dead path (bypasses
production readers) and must never be a debug profile.

1. Add `--profile production|debug` to recorder_host, propagated as one `profile` field in
   `collector_control.json`. Both profiles stay `stream_format=compact`, identical domain
   reader code, identical schemas and fold — only emission/diagnostics differ.
   - production: perf records OFF (or one aggregate frame-level record if gates need a
     count — resolve against `domain_perf_record_count` consumers first), RC counters OFF,
     `compact_country_frame=true`, `lightweight_root_frame=true` **after verifying the
     skipped country-identity scan carries no replay-consumed data**, minimal manifest,
     strict `min_game_hours`.
   - debug: perf records ON, RC counters ON, sidecar phase-timing log ON
     (`sidecar_log`, currently hardcoded false), full manifest, full provenance fields,
     relaxed gates.
2. Update `Start-Hoi4ProductionSpeed5Capture.ps1` to `--profile production`; add a debug
   launcher mode.
3. Audit the control-file keys the DLL parses but the Rust host never writes
   (`army_body_bridge`, `ship_body_bridge`, `war_relation_bridge`, `map_control_*`, …) —
   prune only what is provably dead across BOTH the Rust host and the Python dev harness
   (`extractor/live_capture/`), which may still write them for dev bridge workflows.
4. Fail-closed guarantee is NOT a debug feature: production must preserve skipped-hour
   accounting and per-domain status signaling (SOURCE_UNAVAILABLE / partial-data). What may
   be cut in production is instrumentation cost (per-read RC counters, timing verbosity,
   sidecar logs), never data-loss detection. (Container single-representation moved to
   Phase 3 per D1.)

Expected: a few ms/frame (3.17 M rc_bump calls + perf-record building), container ~2×
smaller immediately (single representation), clean profile surface for everything after.

## Phase 1b — Bounded-memory streaming fold/pack (recorder host) — **DONE 2026-07-09**

Landed (merge f8cc48d6f): streaming capture→disk with rolling SHA-256, mmap manifest gate,
file-backed incremental fold (hour spilled + released at FRAME_END), streaming zip pack with
per-chunk compression, streaming re-read verification, G5 telemetry (`host_finalize.*`) in
every manifest. Proof: streaming fold chain-hash == batch fold_check chain-hash on the real
216 MB stream (`be3d4994…`, independently reproduced by the orchestrator); peak private
1.13 GB and FLAT across 4× stream size (867 MB → +3 MB). Hardening round (Codex review →
Codex fix → re-review → orchestrator direct fixes): shared 2 GiB writer/reader entry cap
enforced mid-stream; zip32 checked casts + entry-count preflight (cap 65,534 — 0xFFFF is the
zip64 sentinel); unexpected pipe read errors now hard-fail instead of publishing a prefix
stream; verifier checks CRC-32/flags/local headers/data descriptors; fail-closed manifest
rewrite when the pack fails (no stale replay advertised). Known deferred: zip64 (needed for
>65,534 entries — a full-game run — goes with the Phase 3 container overhaul); verifier
accepts only our writer's signed-descriptor form (INFO).

### Original defect record (for history)

Defect (see G5): the host's post-capture replay build batch-folds the entire session in RAM
(27–34 GB for a 300 s speed-5 mid-war run) and takes >3× the capture time. Required rewrite:

1. **Fold incrementally during capture:** consume the pipe stream as it arrives, maintain
   only the CURRENT world state + the in-progress hour, emit each finished hour's snapshot
   (or delta) to disk immediately, then release it. Peak memory becomes O(world state), not
   O(session).
2. **Stream the container:** write `.battleplan` entries incrementally (zip supports
   streaming append); never buffer the whole stream or all snapshots in memory. Verify
   entries by re-reading from disk, not from RAM copies.
3. Finalization after the capture ends should be seconds (index/manifest write + container
   close), not minutes.
4. Gate: G5 measured on a live speed-5 run (peak private bytes logged into the manifest),
   plus G1 fold-equivalence — the streaming fold must hash identical to the batch fold on
   the same stream.

This phase is independent of the DLL hot loop and can proceed in parallel with Phase 2.

## Phase 2 — The Big Five: bulk-read collapse (time) + delta emission (bytes)

**Status 2026-07-09:** read-side collapse LANDED for d48 (4-6x fewer reads/object,
byte-identical, all-window-or-all-live fallback), d44 (already collapsed earlier; residual
~22k allocs/frame removed), d41 (pool/lifecycle bulk reads + scalar fallback, payload-equal
fixtures), and d53 W3a (exact-offset relation reader d1f0a5722: the 104-qword/832-byte blob
scan contributed ZERO live emitted facts — real flags are the rel+0xD0 vector — so offsets
mode eliminates ~53 MB/frame memcpy + 6.7M lookups; runtime `d53_relation_read_mode`
offsets|scan|shadow, shadow mismatches fail-close the frame as partial).
**PENDING before next production capture: live shadow A/B for d53** (protocol: one capture
with `"d53_relation_read_mode":"shadow"`, require domain53_relation_shadow_mismatches == 0
on every d53 frame; any mismatch is a blocker; only then consider deleting the scan path).
**Delta-emission wave COMPLETE 2026-07-09** (merge a299d9cc8): all five fat domains now emit
baseline+delta+tombstone — d41 (8fa005b30), d44 (da29ceb90), d48 (963449eb6), d51
(fcb39d4b9, freeze waiver), d53 (75e3419b7). Shared architecture: COMPACT_STATUS_DELTA_DATA
flag, high-bit tombstones carrying only the identity key, per-domain `dNN_emit_mode`
delta|full runtime override, tracker never advances on failure/partial frames (d53: partial
frames emit no deltas at all — no tombstone storms), fold treats non-delta records as full
baselines (old recordings unaffected). Every domain has a lossless gate test folding full vs
delta streams to byte-identical per-hour state; all five pass together. Honest residuals
(stay full-emission, no safe identity): ~~d44 computed badges~~ (CLOSED — see residual-deltas
lane below); d51 delayed-event/scripted-GUI opaque rows (MEASURED-NOT-WORTH-IT — see below);
d53 operation_instances/incoming_actions/intel_network_graphs/factions.

**Residual-deltas lane 2026-07-09 (lane/residual-deltas):** measured both documented d44/d51
full-emission residuals on the deep-war corpus
(`runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin`, 12.9 GB;
4×300 MiB windows @ 0/35/70/~100 % → 28,964 records, 1.225 GB sampled). Per-domain byte
shares of the sampled stream: d53 85.3 %, d48 6.1 %, d20 3.6 %, **d44 1.16 %**, everything
else <0.5 %; **d51 total 0.05 %**.

- **d44 computed badges — CLOSED (implemented + gated).** The five computed-badge rows are
  constant compile-time rows (each wall flag → a fixed `value_a_raw` code); the collector was
  force-emitting all five in EVERY delta frame (sample: 1,305 badge rows over 261 records =
  5.0/frame = **200 B/game-hour**, 0.37 % of d44 bytes, 0.0043 % of the whole stream). Fix:
  the collector now deltas badges like any other row (emit once in the baseline, skip while
  unchanged — they never change) and the fold already RETAINS them
  (`SupplyLogisticsState::badges` keyed by badge_flags; required-badge coverage reads the
  retained set). Also made the host decode gate's mandatory-five-badges check baseline-only
  (a delta record legitimately carries zero badges), mirroring the fold. Gates: collector
  unit test (baseline emits 5, unchanged frame emits 0, changed live row re-emits only itself
  with badges silent); host fold-identity gate updated so delta frames omit badges and folds
  stay byte-identical incl. tombstone/reverted/zero-change frames; decoder tests
  (delta-without-badges decodes clean; baseline-missing-badge still fails).
- **d51 delayed-event/scripted-GUI opaque rows — MEASURED-NOT-WORTH-IT (documented, no code).**
  Two sub-parts, both left full-emission:
  1. *Constant opaque badges* (`VALUE_DELAYED_EVENT_OPAQUE` + `VALUE_SCRIPTED_GUI_OPAQUE`): in
     the corpus exactly 1 + 1 constant badge row per frame (522 rows over 261 records),
     **≈165 B/game-hour**, 6.5 % of d51 bytes but only **0.0035 % of the whole stream**. They
     ARE cleanly deltable in principle (stable string-key identity), but the fold deliberately
     CLEARS `scripted_world_opaque_badges` every delta frame (Vec, rebuilt per-frame), so
     closing them needs a coordinated tombstone-parity change across THREE layers that the
     lane keeps in lock-step — collector key + `world_fold` (Vec→keyed map, stop-clear split,
     upsert/remove, tombstone-flag arm, render) + the FROZEN lane gate
     `validate_tombstone_row` (which today hard-rejects opaque value_kinds) — for a reward the
     zstd-93 container layer (163× measured on exactly this constant-redundancy class) already
     reduces to ≈1 B/game-hour. Verdict: keep full-emission; the container codec is the
     intended absorber (same rationale the plan applies to d53 full-churn).
  2. *Measured delayed-event rows* (`VALUE_DELAYED_EVENT`): carry a per-event
     `due_remaining_hours` that DECREMENTS every game-hour, so a delta would re-emit them every
     frame regardless (zero savings); and they measured **ZERO rows across the entire 12.9 GB
     deep-war corpus** (delayed-event discovery stayed fixture-blocked/unresolved). Verdict:
     keep full-emission.

**Adversarial review wave 2026-07-09 night (Opus reviewers):** bulk-collapse sweep
certified SOUND across all 12 lanes; delta-emission review found and FIXED two real defects
(merge a1c787a4a): d48 tolerated-partial baseline leak (two-accumulator split + phantom-row/
reverted-value regression tests) and d53 operative owner-change duplication
(relocate-on-upsert). Coverage gate verified fail-closed on missing/duplicate per-hour data
records. Live session is now one command: scripts/Invoke-LiveValidationSession.ps1.

**Remaining in Phase 2:** LIVE validation session — (1) d53 shadow gate
(`d53_relation_read_mode:"shadow"`, zero mismatches required), (2) paired speed-5 A/B vs the
07-09 baseline (frame avg/p95, skipped hours, bytes/game-hour, host_finalize peak RAM),
optionally with per-domain emit-mode full/delta A/B for size attribution. d51 bulk-read
collapse (read-side) is still open if the A/B shows it matters.

Per lane (order: d48 → d44 → d41 → d53 → d51), two changes, each gated by G1:

- **Bulk collapse:** replace per-field guarded reads with few large `read_bytes` of the
  contiguous object/array, parse in memory (weather/P4c-2 pattern; `ObjectSnapshot`
  substrate `62d9643ac` exists). Repo-bounded estimate 2–4× per lane; the ~90 ms these
  lanes occupy deep → expect **−40 to −60 ms**.
- **Delta emission:** baseline once + changed-row deltas + tombstones, exactly like
  divisions/map_control. d53 needs its fail-closed "clean frame full-replaces" semantics
  rewritten delta-safe (`compact_contract lib.rs:156`). Expected stream **~10–15× smaller**
  (the five lanes are 93.6 % of bytes; peer lanes emit 0.3–0.7 % changed rows).
- **d53 exact-offset relation RE (W3a):** replace the 832 B blob × 104-qword × ~64 k-pair
  scan (~6.7 M BTreeMap lookups + 53 MB memcpy per frame) with reads of the known flag
  offsets (rel+0xD0 vector already RE'd, subject-ptr +0x288 pattern). Est. d53 relations
  ~75 → ~15–25 ms. Needs RE validation packet + save-oracle equivalence. First scope the
  ~40 ms "dark residual" outside d53's phase timers with success-only outcome-split timing.
- **Prefetch sweep** (dist {8,16,32,64} via collector_control) extended to the cold walks;
  d53 measured −17 % from dist-16.
- d51 is a FROZEN lane → needs the freeze-waiver decision (D2) before touching; d47/d52/d56
  (another ~40 ms deep combined) likewise if Phase 4 needs them.

Phase target (G2): deep-war warm avg **≤ 45 ms** speed-5 zero-skip; stream ≤ ~0.7 MB/game-hour.

## Phase 2c — Offline read-collapse sweep COMPLETE 2026-07-09 (late)

Beyond the Big Five, bulk-read collapse (byte-identical contract, all-window-or-all-fallback,
equivalence + refused-window fixtures per lane) landed for EVERY deep-war lane above ~13 ms:
d51/d52/d56 frozen lanes (627a25f87/60c035914/586666f09), d23 map_detail + d26
division_equipment (35790f468/30d62ed9d), d28 oob_command + d29 ships (376c9e50f/14e84996c),
politics_live (921c7c9fa). All merged with green suites. Live A/B still pending for actual
ms gains. stream_fold gained --discard-snapshots (688ca2840) so full-stream soaks fit disk.

## Phase 3 — Container & codec overhaul (bytes) — CORE LANDED 2026-07-09

zstd method-93 (level 13, window_log 27, LDM) + full zip64 shipped in the streaming writer
and both readers (222fae43f + fix round f2631ca70; adversarial-review verdict SOUND, all 5
findings fixed: streamed file-backed entries, reader allocation caps, EOCD-at-EOF
validation, checked-in pre-222 legacy fixture, configurable CompressionProfile).
**Measured on the real 28 GB deep-war stream (2 GiB slice): zstd 163.2x @ 483.5 MiB/s vs
deflate-6 9.37x @ 52.6 MiB/s.** Legacy deflate containers remain readable. Remaining Phase 3:
snapshot-term elimination via the D1 shared decoder crate (in progress — replay_core
consumes the stream directly; snapshots move to debug profile), keyframe/binary snapshot
format only if the decoder-crate route stalls.

## LIVE VALIDATION SESSION 2026-07-09 NIGHT — results of record

Run: runs/live_validation_20260709_221706 (+ probe in live_validation_20260709_215202).
- **d53 SHADOW GATE PASSED**: domain53_relation_shadow_mismatches == 0 on every frame with
  both paths live. The exact-offset relation reader is live-proven; the legacy scan may be
  retired next cleanup.
- **Light save probe** (restarted game, ~13.5 h/s): frame avg 36.4 ms, 1/162 hours skipped
  (cold frame), ~0.69 MB/hour (~17x smaller than baseline), host peak 604 MB.
- **Deep late-war save** (~6 h/s, the game 8.2 GB): frame avg 151.5 ms p95 263.6, 370/1,790
  hours skipped (20.7%), 12.9 GB/300 s raw (~9.1 MB/hour). NOT a paired A/B vs the morning
  baseline (different save depth — 21 h/s vs 6 h/s); it maps the remaining gap on heavy
  worlds. CONCLUSION: **Phase 4 (lossless change-driven capture) is formally necessary**
  for deep-war sub-20 ms; read-collapse+delta alone do not defeat work-scales-with-world.
- Delta effectiveness on deep war: d51 0.3% changed, d41 8.3%, d44 10.0% (all working);
  **d48 79.2% — ROOT-CAUSED + FIXED (lane/d48-partial, see below)**; **d53 ~full-churn** —
  genuine hourly opinion drift; the zstd-93 container layer (163x measured on this redundancy
  class) is the intended absorber, and per-field delta encoding is a candidate follow-up.

### d48 tolerated-partial churn — RESOLVED (lane/d48-partial)
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
Fix (COLLECTOR side, composes with fold fix a1c787a4a): added a second collector accumulator
`partial_projection` mirroring the fold's `domain48_state`. A tolerated-partial frame now emits
only the read rows that differ from the projection (upserts only, **no tombstones** — an omitted
row may be unread, not deleted), and sparse-merges the read rows into the projection. The DELTA
baseline (`last_*`) is left pinned across partials (unchanged, still mirrors the fold's
`domain48_delta_state`), so the next real delta and the fold stay byte-identical. The projection
is diffed against (not the pinned baseline) so a value that a prior partial pushed and then
reverted is still corrected (no data loss). The overlay is `None` on the common all-clean/delta
path (no per-frame clone; one clone per partial run, cleared by every clean/delta frame).
Gates (all automated, all green): collector unit tests
(`attach_log::domain48_partial_emit_tests::*`, 7) — churn mutation (unchanged partial emits 0
rows vs legacy 3; one-change partial emits 1), baseline never advanced on partial, reverted-to-
baseline value re-emitted via projection diff, partial never tombstones, delta still tombstones
real removals, projection reset, full-mode intact. Fold identity test
(`compact_d48_reduced_partial_folds_identically_to_full_emission`) proves the reduced-partial
delta stream folds byte-identically to full emission AND to the legacy full-read-partial stream.
The a1c787a4a phantom-row/reverted-value regression still passes unchanged.
Measured churn on the synthetic fixture: unchanged tolerated-partial hour 3 rows → **0** (was 3,
full re-emit); genuinely-changed subset emits exactly the changed rows.
- **New bottleneck of record: the post-capture fold** (~5 MB/s single-threaded serde_json)
  — 12.9 GB folds for ~40 min. Validation sessions killed it after manifest write (numbers
  live in the manifest). Fix candidates: parallel per-hour fold, faster canonical
  serialization, or skip host fold in production once replay_core consumes the stream via
  replay_decoder (D1). Session tooling: --replay-zstd-level plumbed (validation uses 3);
  emit-mode-full stage optional.

## CORPUS RETENTION (2026-07-10 — disk cleanup, user-authorized)

Deleted, to reclaim ~83 GB: the orphaned partial-fold snapshot temp dir (34.6 GB, regenerable
from the stream), seven merged-lane worktrees (15.9 GB, recoverable from git), the
**morning-baseline stream** (26.2 GB), and **all `live_perf_20260708/09` streams** (11.5 GB).

**All 54 run manifests were retained and are now the numbers of record for those runs.** Read the
manifest; there is no `.bin`. Morning baseline stays quotable as 69.9 ms avg / 82.4 p95 / 3,897
skipped / 28.15 GB — but remember it was a *lighter world* and is **not** directly comparable to
the deep-war run.

**Surviving stream corpus (the only one):**
`runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin` — deep-war,
~12 GB, 1421 hours. Reference fold chain hash over its first 600 MiB:
`bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7` (198 hours, 22,059 records),
reproduced identically by sequential and 4-worker parallel folds. **There is no backup, and
re-capturing any deleted stream requires a live game session.** Read it in place; never copy or
modify it.

## ⚠ CORRECTION 2026-07-10 (b) — skip law re-measured; "we cause the bursts" REFUTED

Full study: `lane_reports/burst-correlation.md`. Header-only scan of the deep-war stream, using
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
- **Unresolved confound (do not drop):** the curve assumes the game's pace is exogenous. The
  `+0.4364` correlation says it may not be, and observation cannot separate common-cause from
  causation. **D≈28 ⇒ zero skips is an upper bound, not a prediction.** Settle it live: hold save +
  speed fixed, vary sweep cost via domain subsets (~40/~80/~151 ms), and see whether the *interval*
  distribution shifts. Unchanged ⇒ exogenous; shrinks with us ⇒ CPU-coupled fixed point.
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

## ⚠ OUTSTANDING INDEPENDENT GATES (2026-07-10)

Per `CODEX_LANE_BRIEF.md` §7, a lossless-critical diff needs the **orchestrator's** independent
adversarial gate on the final state — the implementer never clears its own work. Two such diffs
landed on 2026-07-10 and **that gate has NOT been run**: Codex was rate-limited
("hit your usage limit") on every attempt. It is not run, so it is not claimed.

| Diff | Ships as | Orchestrator-verified | Independent gate |
|---|---|---|---|
| `hour_latch` | **default OFF** (`unwrap_or(false)`; no thread, no extra reads) | scope, fold-path zero edits, 1203/0 tests, G1 chain hash exact | **NOT RUN** |
| `d53 delta_partial` | **opt-in** (default `delta` byte- and CPU-identical) | scope, 1214/0 tests, **G1 chain hash exact despite `world_fold.rs` edits** | **NOT RUN** |

Both are inert by default, which is why they were allowed to merge. **Neither may be ENABLED**
until (a) the independent adversarial gate runs clean, and (b) their live gates pass (hour-latch:
shadow-soak + paired A/B + 6-core run; d53: paired A/B confirming the +46–68 ms/frame cost).

Paste-ready gate prompts + the next lane (three-phase timing instrumentation) are queued in
`CODEX_DISPATCH_QUEUE.md` — dispatch the moment Codex quota returns; nothing needs re-derivation.

Note the lanes' *own* Codex reviews did run and were valuable: hour-latch's returned REFUTED and
exposed a real bug (a stale sweep read fabricated a false reload + duplicate capture); d53's
returned REFUTED and exposed **two real silent-data-loss bugs** its own fuzzer had missed
(empty-first-frame baseline desync; mutate-before-validate). That is the case for reviewer
independence, in evidence.

## ⚠ WHAT "ZERO DATA LOSS" ACTUALLY MEANS (2026-07-10 — scope correction)

The `lane/hour-latch` lane forced this, and it is load-bearing for the whole program.

**An async sampler of a sim it cannot pause cannot deliver literal zero data loss.** Two facts:

1. **Intra-hour transients are unobservable to any hourly sampler.** A value that changes and
   reverts within one game-hour is invisible no matter how fast the sweep is. Hourly cadence is
   the requirement; hourly cadence is also the ceiling on what can be seen.
2. **A skipped/coalesced hour IS real loss of that hour's world state.** The status quo already
   loses hours: 370 of 1790 (20.7%) on the deep-war run. The latch does **not** fix that — it
   makes it *accounted* rather than *silent*.

So the achievable guarantee, and the one we now hold ourselves to, is:

> **ZERO-SILENT-GAP ACCOUNTING + BOUNDED SKEW.** Every game-hour is observed and accounted as
> either a capture attempt (confirmed-complete or recorded-failed) or a **reason-tagged**
> coalesce (superseded / overflow / flush / backward / anomalous). Invariant:
> `observed == attempts + coalesced + in_flight`, with `in_flight == 0` after flush. Read skew
> exists as it does today and is bounded, not monotonically improved.

**Capturing every hour remains the goal — and it is a SWEEP-SPEED problem, not a latch problem.**
Under sustained overrun only ~`RUN_MS / sweep_ms` hours can be captured; the rest are physically
unrecoverable. The latch buys correct accounting and absorbs *momentary* bursts (which is what the
28.2 ms bursts against a 167.6 ms mean actually are). Getting hours captured rather than coalesced
is Phase 2/4's job: make the sweep fast.

**Crucially, silent loss is prevented by DELTA accounting, not by the poll rate.**
`HourLatch::observe` accounts every hour in `last+1..=raw`, so a multi-hour jump between polls is
detected and coalesced-with-reason. 28.2 ms is the minimum *observed* interval in one capture, not
a proven floor — correctness must never depend on it, and does not.

### Status: MERGED but NOT CLEARED FOR ENABLING
`hour_latch_enabled` defaults to **false** (`unwrap_or(false)`); no thread is spawned and no extra
game memory is read when off, so legacy behaviour is unchanged. Verified on the merged tree by the
orchestrator: fold path has **zero edits**, workspace **1203 tests / 0 failed**, and G1 fold chain
hash on the deep-war 600 MiB prefix reproduces the reference
`bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7` exactly.

**OUTSTANDING before it may be enabled:**
1. **The orchestrator's independent adversarial gate DID NOT RUN** — Codex hit its usage limit.
   This is a lossless-critical diff, so per `CODEX_LANE_BRIEF.md` §7 it requires that gate on the
   post-fix state. *Not yet performed.* (The lane's own Codex review did run and returned REFUTED
   on the first draft; one fix round was applied — see `lane_reports/hour-latch.md` for verbatim
   findings, incl. a real bug where a stale sweep read fabricated a false reload + duplicate
   capture.)
2. Live shadow-soak: observed matches the counter 1:1, invariant holds throughout.
3. Paired A/B vs legacy: skip count, frame p95, max_queue_depth, RAM.
4. 6-core portability companion run.

## ⚠ CORRECTION 2026-07-10 — the Phase 4 "1.2 ms floor" is UNSUPPORTED

The `lane/phase4-mirror-mapdetail` lane (report: `lane_reports/phase4-mirror-mapdetail.md`,
Codex-reviewed twice) **refuted the premise** that `map_control`'s 40x advantage comes from a
dense-mirror / change-driven read. Verified independently against the source:

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

The live open question for scattered-object domains is **region-coalesced bulk reads** of
already-known pointers (complete by construction; cuts guarded-read *calls*, not entities).
Payoff is an empirical address-locality question, unanswerable offline — run
`runs/d23_speed/d23_locality_probe.py` on a paused deep-war save before building anything.

## DEEP-WAR LEVER RANKING (2026-07-09 night corpus)

> **Read the correction above first.** Lever 1 and its sub-rows (1a–1g) quote a 1.2 ms
> change-driven floor that is now known to be data-shape-dependent, not universally reachable.

Source: `runs/live_validation_20260709_221706/stage2_shortspeed5` — manifest per-domain
`read_time_us` + a full header-only scan of the 12.0 GB `collector_stream.bin` (1421 frame_end
records for per-frame `frame_time_us`, plus per-domain DOMAIN_DATA payload bytes from the H4CR
headers; scan script in lane scratchpad, read-in-place). Frame avg 151.5 ms, p95 263.6, max
477.2; 1421 hours captured / 370 skipped of 1790 advanced (20.7%); 9.068 MB/hour (matches the
9.08 reported to 3 s.f.). Numbers below are that run only.

### 1. Per-domain frame-time ranking — and the parallelism caveat

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

**MISSING COUNTER (propose as a lever):** the manifest carries per-domain wall but NO per-frame
per-lane / critical-path counter, so frame-ms cannot be attributed to lanes and we cannot see
which domains co-schedule on the slow path. Every ms-saved estimate below therefore carries a
parallelism band: work removed saves between `X/4.58` (if fully hidden behind other lanes) and
`X` (if on the critical path). Emitting a per-frame lane-assignment + lane-wall in the frame_end
record is a zero-risk instrumentation lever that unblocks exact attribution.

### 2. Skip mechanism — the premise is only half true

The skip law is `skipped = Σ(gap−1)` over consecutive captured hours (`main.rs:16575`) on a
**free-running game the extractor samples** (read-only; extraction does NOT gate the sim). Skips
are **weakly** correlated with frame time:

- 139 of 370 skipped hours (38%) follow frames **under 120 ms**; the 150–168 ms band skips the
  *least* (14%). A gap=2 skip occurred at the run's **fastest** 59.5 ms frame.
- Cause: the game's **local tick rate is bursty**. Reconstructed local inter-hour interval
  (`ft/gap`): p50 129.5 ms but **min 28.2 ms, p1 33 ms, p5 53 ms** — the sim bursts to ~22–35
  effective h/s even though the 300 s average is 5.967 h/s (167.6 ms/hour). A frame skips
  whenever it exceeds the *instantaneous* interval, so any frame landing in a burst skips.

**Zero-skip frame-time threshold (this run): ≤ ~25 ms** (every observed local interval ≥ 28.2 ms;
at a 25 ms cap 0/1420 windows are tighter). Practical caps leave residual skips: ≤100 ms → 28.2%
of windows still tighter, ≤60 ms → 7.7%, ≤50 ms → 3.9%. **No practical uniform frame time yields
true zero-skip on a bursty deep-war world** — reaching it needs either sub-25 ms frames
(unreachable for this domain set) or an architectural change that decouples the cheap hour-boundary
latch from the heavy decode. The 167.6 ms "average budget" framing is wrong; the binding number is
the min-burst interval. Reducing avg/p95 frame time still cuts skip *count* materially (a ~50 ms
frame floor would take 20.7% → low single digits), just not to zero.

### 3. Where the 9.068 MB/hour lives (post-delta)

Header-summed DOMAIN_DATA bytes/hour: **d53 diplomacy_intel_operatives = 8.372 MB = 92.3%** of the
entire stream. Everything else is noise: d48 construction 288 KB (3.2%), divisions 169 KB (1.9%),
supply 52 KB (0.6%), capitulation 35 KB (0.4%). Delta cross-check (changed_rows/row_count,
cumulative) reproduces the reported figures: d51 0.3%, d41 8.3%, d44 10.0%, **d48 79.2%**, and d53
churns >100% (242 M opinion rows re-emit hourly). The delta wave works everywhere except d53 and
d48 — **bytes are now a d53-only problem** (zstd-93 at 163× is the intended absorber; per-field
opinion delta is the structural follow-up).

### 4. Ranked levers (ms saved on THIS corpus ÷ risk)

Change-driven projection uses the **empirically proven floor**: map_control reads the *same*
20.6 M-row province set as map_detail but costs **1.2 ms vs 49.2 ms (40×)** because it is already
change-driven — so `proj = max(1.2 ms, changed_frac × read)`.

| # | lever | est. read-work cut | frame effect | evidence | risk | owner |
|---|---|---:|---|---|---|---|
| 1 | **Phase 4 change-driven reads** (whole program) | **532 ms of the 693.6 ms read-sum (77%)** | critical path 151.5 ms → **~50–70 ms floor** (set by d53 49 ms + divisions 24 ms + d48 18 ms, the only genuinely-changing heavies) | map_control 40× proof; 9 domains at 0.1–2.2% changed re-read the full world | HIGH (needs RE'd dirty signal + shadow-soak fold-identity gate, D3) | Phase 4 (unassigned) |
| 1a | ├ oob_command change-driven | 58.6→1.2 = **57.4 ms** | on slow path (3rd heaviest) | 0.7% changed, 1.4 M roots for 9 k rows | HIGH | Phase 4 |
| 1b | ├ map_detail change-driven | 49.2→1.2 = **48.0 ms** | slow path | 0.1% changed; map_control already does it | MED (twin exists) | Phase 4 |
| 1c | ├ supply_44 change-driven | 60→6 = **53.9 ms** | slow path (heaviest) | 10% changed | HIGH | Phase 4 |
| 1d | ├ ships change-driven | 57.9→5 = **52.9 ms** | slow path | 8.7% changed | HIGH | Phase 4 |
| 1e | ├ division_equipment | 41.9→1.2 = **40.7 ms** | slow path | 2.2% changed, 23 M entry rows | HIGH | Phase 4 |
| 1f | ├ politics_live | 37.9→1.2 = **36.7 ms** | mid | 0.6% changed | HIGH | Phase 4 |
| 1g | └ occupation_52 / naval_missions_56 / scripted_51 | ~24+24+22 = **~70 ms** combined | mid | 1.3% / 0.8% / 0.3% changed | HIGH | Phase 4 |
| 2 | **d48 churn fix** (79.2% → ~10%) | ~22→~3 = **~19 ms** + 260 KB/hr bytes | mid lane | 79.2% changed is implausible for construction hourly; neighbors 0.1–10%. Suspected tolerated-partial tracker re-emitting vs stale baseline | **LOW** (no RE; fold-identity gated; collector sparse-merge advance) | flagged INVESTIGATE (night note) |
| 3 | **d53 per-field / opinion-change delta** | read: relations phase **43.2 ms/frame = 88% of d53**; bytes: **8.37 MB/hr = 92.3%** | sets the post-Phase-4 frame floor AND the container size | d53 stays 100%+ churn so Phase 4 dirty-signal does NOT help it; needs per-opinion change-stamp | MED–HIGH (opinion delta correctness; bytes already 163× absorbed by zstd so read-time is the real prize) | d53 lane |
| 4 | **W1.1 prefetch sweep {8,16,32,64} + success-only outcome-split timing** | single-digit ms on d53 (attributes the ~37% dark residual) | small on slow path | W0 already banked −43% (190→109 ms warm), byte-identical; W0_AB_RESULTS §5 | LOW–MED (CPU-only, fixture-gated) | W-series perf lane |
| 5 | **Per-lane critical-path counter** | 0 (instrumentation) | unblocks exact ms-attribution for #1–#4 | frame≠Σread (4.58× gap) is currently un-attributable | **NONE** | recorder/collector |
| 6 | **divisions partial change-driven** | 41.3→24.2 = **17 ms** (the unchanged 41%) | slow path | 58.6% genuinely move late-war; only the static 41% is addressable | HIGH (per-division dirty) | Phase 4 |
| 7 | **naval_combat walk (d49+d50 shared)** | 34.1→~6 = **~28 ms** (one walk, two domains) | mid; already fail-closes 367 torn frames | 4.4%/19.3% changed on a 34.2 M-µs CFEXGroup walk | HIGH (torn-read correctness) | naval lane |

### 5. Irreducible fraction without Phase 4

**77% of the read-work (532 of 693.6 ms) is spent reading provably-unchanged rows to prove they
are unchanged.** Delta emission already removed those *bytes*, but the *reads* still happen —
delta is byte-lossless, not read-free. Without Phase 4 the only remaining read levers are banked
(read-primitive + bulk-collapse, 3.57×) or per-domain point fixes (d48 churn, d53 W0/W1.1 CPU),
which leaves deep-war at ~100–150 ms. **Sub-20 ms deep-war is unreachable without a change-driven
read path** — this quantifies the "Phase 4 formally necessary" conclusion already recorded above:
the 151.5 ms frame is ~77% unchanged-proving read work, and only a dirty-signal that lets those
reads be skipped (D3, soak-gated) removes it.

### Top recommendation for the wave after this one

**Stand up Phase 4 change-driven reads behind ONE shared RE'd dirty-signal, sequenced by cut/churn:
land the low-churn heavies first (map_detail, oob_command, politics, division_equipment,
occupation_52, naval_missions_56, scripted_51 — all <2.2% changed, each collapsing to the ~1.2 ms
map_control floor), gated by a shadow-soak fold-identity harness (D3/G1).** Run two cheap lanes in
parallel: (a) the **d48 churn fix** — low-risk, no RE, also resolves a data-quality suspicion; and
(b) the **per-lane critical-path counter** — zero-risk instrumentation that makes every subsequent
ms-saved claim exact instead of parallelism-banded. Treat **d53** (opinion-change delta) as the
long pole with its own lane: after Phase 4 it is simultaneously the frame-time floor (49 ms, 88% in
the relations read) and 92.3% of stream bytes. Finally, reset expectations on skips: Phase 4 buys a
~50–70 ms floor that cuts skips from 20.7% to single digits but **cannot** reach true zero-skip on a
bursty world (min local interval 28.2 ms) — true zero-skip is a separate architectural item
(cheap hour-boundary latch decoupled from heavy decode), not a frame-time-reduction item.

### lane/fold-perf — post-capture fold bottleneck (2026-07-10) — **LANDED, 5.4x measured**

Profile first (FOLD_PROFILE=1 env-gated phase timers, deep-war 512 MiB prefix, 189 hours,
release, this rig): fold wall 173.1 s = **build_snapshot 65%** (75.2 s), apply_domain_data
10.5% (12.2 s), serde_json serialize 10% (11.5 s), snapshot write 3.8% (4.4 s). Input
512 MiB expands to 5.13 GB of per-hour snapshot JSON (14x amplification, 27–55 MB/hour) —
serialization/IO were never the wall; per-hour `Value`-tree projection is.

Levers landed (all G1-gated byte-identical, all orchestration-layer — zero domain-fold
body edits):
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

Also landed (separate commit): `--skip-host-fold` production option — skips the host fold
entirely (raw compact stream is the canonical artifact; replay_decoder D1 gives consumers
per-hour access), packs an empty snapshots array, and marks the manifest + container index
`host_fold_skipped` explicitly. Default behavior unchanged.

## Phase 3 (original spec below)

1. **zstd with long window** for container entries (writer `snapshot/src/pack.rs:456` is
   deflate-6; `zip.rs` supports STORE/DEFLATE only → new method or outer frame). Measured
   proxy: lzma-6 42.9× vs deflate 8.07× on the real stream. Add the zstd crate and measure
   for real (G4).
2. **Snapshot term** (72 % of container): keyframe + delta snapshots and/or binary columnar
   snapshot format (hour-to-hour snapshots ~99 % identical; measured concat-lzma 24.7× vs
   15.6× per-file). This changes the consumer contract in
   `replay_interface/native/replay_core/src/source.rs` — replay_interface is another
   agent's lane, so this goes through `EXTRACTION_INTERFACE_REQUESTS.md` (see D1).
3. Varint/bit-packing of rows (48–81 % zero bytes measured) only where it survives
   post-zstd measurement — likely subsumed by the codec.

Phase target (G4): **low-single-digit GB per full game** (from ~300 GB), production profile.

## Phase 4 — Structural time floor: lossless change-driven extraction

**STATIC-RE SCOUT COMPLETE 2026-07-10 (`lane/phase4-scout`) — see
`extractor/documents/PHASE4_DIRTY_SIGNAL_SCOUT.md` (full evidence, addresses, decompile
snippets). Headline results that re-shape this phase:**

- **No engine-native per-object dirty/version signal exists.** Confirmed by static RE of
  `hoi4.exe` (Ghidra, image base `0x140000000`): the relation opinion writer at `0xd09d50`
  bumps no version; `CUpdateable` (`updateableinterface.cpp`) is a pure-abstract base with no
  dirty bit; the only `dirty` strings in the binary are UI tree relayout. The MP desync
  **checksum is a monolithic O(entire-world) serialize walk** (`0x1f2a20` size 8376 / per-
  category mgr `0xd96cb0` size 12878), computed on demand, a single hash — **as expensive as
  the full read and unable to say which row changed. Do NOT build Phase 4 on the checksum.**
- **The completeness-safe lever is NOT "read only what changed" but "read every value cheaply
  via the engine's dense mirror arrays + diff host-side"** — this is *complete by construction*
  (no signal-miss / zero-loss safe) and is exactly what `map_control` already does live at 40×
  vs `map_detail` (dense `province_array_ptr + id*stride + controller_ptr_offset` read; delta is
  host-side in `world_fold.rs`, NOT an engine signal). Phase 4 RE work = **per-domain mirror
  discovery** for the low-churn heavies (`map_detail` first — its twin proves the mirror is
  real). Where no mirror exists (d53, >100% churn), no change signal helps → per-field lane.
- **Zero-skip latch (orthogonal to "what changed"):** main-thread hour-boundary hook site found
  `postHourlyUpdate`, reads the hour counter as `(gs[0x468]-43800000) % 24`, asserts
  `ThreadIsMainThread()`, fans out via TBB. A non-mutating atomic-store detour at
  `postHourlyUpdate` latches every hour (never missed) decoupled from the heavy decode; risk
  HIGH (main-thread sim patch → MP-checksum control gate required). See scout doc §Hour-latch.
- **Fallback if a heavy has no mirror:** OS `GetWriteWatch`/`PAGE_GUARD` — complete but 4 KiB
  page-granular and hostile to the game's shared-page heap layout; low selectivity, last resort.

Original scoping (superseded on the dirty-signal question by the scout above, retained for the
bulk-rewrite fallback):

Bulk-collapse's theoretical floor: converting 3.17 M scattered reads into ~10² region-sized
reads (~60 MB/frame) costs single-digit ms of memcpy + parse; whether parse can be made
cheap enough (SIMD/vectorized row extraction) decides if full-walk alone reaches <20 ms.
Measure after Phase 2 — if deep-war is still >20 ms:

- **Engine dirty/version signal RE** — **RESOLVED: none exists (scout 2026-07-10 above).** A
  skip is legal only for provably-unchanged objects; since no engine signal proves that, the
  sanctioned path is the dense-mirror cheap-read + host-diff (complete by construction). A skip
  is legal only for provably-unchanged objects. Validation: long shadow soaks where change-
  driven and full-walk paths run side-by-side and must fold identical for the entire session,
  plus save-oracle checks, before it may ship (G1 at soak scale) — soak design detailed in the
  scout doc §LIVE VALIDATION PLAN. If no mirror is found for a walk, the fallback is deeper
  bulk-rewrite of that walk + frozen-lane waivers.

Final target (G2/G3): deep-war warm avg **<20 ms** (internal 12–15 ms) speed-5 zero-skip on
the dev rig; hourly gate holds at 6 cores.

## Instrumentation — per-frame lane-wall counter (deep-war lever #5) — LANDED 2026-07-10

(Status note: the "DEEP-WAR LEVER RANKING" section this lever was ranked in is not present
in this pinned revision, so the landing is recorded here.)

WHY: deep-war analysis (2026-07-09 night) measured sum(per-domain avg `read_time_us`) =
693.6 ms vs frame avg 151.5 ms at reader_concurrency=12 (~4.58x effective parallelism). The
frame is CRITICAL-PATH bound, but nothing recorded which domains shared a reader lane or
which lane's wall bounded the frame, so every per-domain ms-saved claim carried a 1x–4.58x
attribution band. This lever removes that blindness. Instrumentation only — zero
capture-semantics change, all profiles.

WHAT LANDED (branch `lane/lane-wall-counter`):
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

## Phase 5 — Acceptance & soak

Full-game-length soak (or the longest practical session): speed-5 zero-skip, deep-war,
production profile; report frame avg/p95/max, bytes/game-hour, projected full-game size,
fold-equivalence hash chain, plus the 6-core companion run. Numbers, not adjectives.

## Decisions — RESOLVED by user 2026-07-09

The stated end goal: hand the `.battleplan` to the replay layer and watch the entire game
play out in absolute detail; the replay layer is being built as a reverse-engineered game
engine that consumes the extracted data directly (read-only playback). Full architectural
free reign granted, bounded only by the zero-data-loss requirement.

- **D1 ADOPTED — stream is canonical.** The compact typed stream is the full-detail record
  the RE'd engine will eventually consume directly. Production migrates to stream-only
  containers once a shared decoder crate exists for replay_core (coordinate via
  `EXTRACTION_INTERFACE_REQUESTS.md`); until then production containers keep the folded
  snapshots so recordings stay renderer-consumable. The container-representation change is
  therefore Phase 3 work, not Phase 1.
- **D2 GRANTED — freeze waiver.** Frozen lanes (d51; d47/d52/d56 if needed) may be
  rewritten under a fold-identical gate (stream bytes may change; decoded state must not).
- **D3 CONFIRMED.** Change-driven capture with an RE-proven dirty signal + soak-validated
  fold-identity counts as lossless.

Non-negotiable consequence of the end goal: the production stream must always remain a
complete, self-sufficient full-detail record — no lever may ship that makes the stream
lossy relative to today's decoded state.

## Known nulls — do not revisit

- Reader-thread concurrency tuning (flat 54–62 ms across rc 0→24).
- "Fewer guarded reads" without a mechanism (cheap MRU-hit reads cost ~nothing; here the
  3.17 M-read volume IS expensive at ~34–320 ns/read, which is why bulk-collapse works).
- Read-primitive swaps (SEH + region cache already banked, 3.57×).
- JSON stream format as a debug profile (divergent dead path).

## lane/d53-perfield — 2026-07-09 (TASK 1 shipped; TASK 2/3 numbers-driven DEFER)

Assigned: (1) make d53 exact-offset relation reader the unconditional default and strip
per-frame scan cost; (2) per-field delta for relation rows; (3) row-level delta for the d53
full-emit residual subdomains. Measurement on the REAL deep-war stream re-scoped 2 and 3.

**TASK 1 — SHIPPED.** `offsets` is already the default in the collector, host
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

**DECISIVE MEASUREMENT (reframes TASK 2 + TASK 3).** Tool
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

**TASK 2 (per-field delta) — DEFER, documented with numbers.** Per-field delta only applies
on delta frames, which never fire in deep war, so in production it saves **0 bytes** today.
Even hypothetically (if delta fired): per-field would omit the unchanged non-key field bytes
= **33.6 % of changed-row bytes** (opinion 25 %, state_event 37.5 %, agency 83 %, operative
79 %, cluster/subject 50–67 %). But d53's dominant rows are key-dominated — OpinionRow is
key(8)+opinion(4) and StateEventRow's two value fields co-change — and changed rows are ~0 %
of the frame, so per-field's ABSOLUTE contribution to d53 volume is ~0. Implementing a
versioned per-field wire format across encoder/decoder/fold/collector would add per-frame
field-compare CPU (violating the portability law) for ~0 production benefit and expand the
losslessness attack surface. **Not implemented; it is strictly downstream of enabling delta
on partial frames (below), and marginal even then.**

**TASK 3 (row-level delta for full-emit subdomains) — DEFER, "not worth it" with numbers.**
Per-frame wire volume of the named residuals: `factions` **~408 B/frame** (changes only
2/254 frames), `operation_instances` **~77 B/frame** (3/254), `incoming_actions` ~0,
`intel_network_graphs` / `operation_anchors` / `intel_map` **EMPTY** (0 frames present).
Combined **<500 B/frame vs a 5.9 MB frame = 0.008 %**. Row-level delta would save ~99 % of
that (~480 B/frame) but only on delta frames, which never fire. Absolute saving is
negligible; not implemented.

**REAL LEVER (correctly-scoped follow-up): enable delta on tolerated-PARTIAL frames.** This
is the only change that saves real d53 bytes. It is a SEMANTICS redesign, not a plumbing
toggle — the current partial-frame fold semantics must change with it:
`domain53_country_overlays_this_hour` is CLEARED at every hour reset (`world_fold.rs:1431`),
so in a deep-war hour (one partial frame) today's snapshot = that frame's observed set:
torn rows are silently DROPPED from the hour, and in-game deletions are conveyed only
implicitly by the full re-emit. A baseline-seeded delta variant must therefore define, and
prove, per-key semantics: (a) collector diffs observed rows vs the last non-partial
baseline (two-accumulator: the baseline does NOT advance on partial frames) and emits
`DELTA|PARTIAL` with changed/new observed rows; (b) POSITIVE-absence tombstones only — the
relation walk distinguishes slot==0 (positively absent → tombstone is safe) from a torn
read (unknown → NO tombstone; fold retains baseline value, which is BETTER than today's
drop-for-the-hour but is a deliberate semantics change to document); (c) fold accepts
`DELTA|PARTIAL` by seeding the hour from the delta baseline + upserting, WITHOUT advancing
the baseline (replaces the `delta_partial_unsupported` rejection at `world_fold.rs:10530`);
(d) automated gate: fold(delta stream) byte-identical to fold(full stream) under the SAME
new partial semantics, plus randomized-sequence round-trip property tests. Projected from
the measured stream: d53 **~5.9 MB/hour → tens of KB/hour (>99 % reduction)** at zero data
loss, and it also deletes most of the ~39 ms/frame payload-encode CPU (the W1.1
`seg_payload` residual) since only changed rows are encoded. This is a losslessness-
critical redesign of the fattest domain and must be its own gated lane (the losslessness
proof is offline — fold-identity over recorded/synthetic streams, no live capture needed).
Per-field delta (TASK 2) becomes a marginal add-on only after this lands.
