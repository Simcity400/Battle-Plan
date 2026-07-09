# Extraction Optimization Plan — sub-20 ms frames + lean replays (2026-07-09)

Owner: extraction lane. Status: PLAN (approved targets, implementation not started).
Codex CLI is unavailable until the user re-enables it; no step below may depend on it.

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
(stay full-emission, no safe identity): d44 computed badges; d51 delayed-event/scripted-GUI
opaque rows; d53 operation_instances/incoming_actions/intel_network_graphs/factions.

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
  **d48 79.2%** — suspect tolerated-partial tracker stalls re-emitting vs stale baselines
  (INVESTIGATE; possible fix = collector sparse-merge advance on tolerated-partial,
  keeping fold identical); **d53 ~full-churn** — genuine hourly opinion drift; the zstd-93
  container layer (163x measured on this redundancy class) is the intended absorber, and
  per-field delta encoding is a candidate follow-up.
- **New bottleneck of record: the post-capture fold** (~5 MB/s single-threaded serde_json)
  — 12.9 GB folds for ~40 min. Validation sessions killed it after manifest write (numbers
  live in the manifest). Fix candidates: parallel per-hour fold, faster canonical
  serialization, or skip host fold in production once replay_core consumes the stream via
  replay_decoder (D1). Session tooling: --replay-zstd-level plumbed (validation uses 3);
  emit-mode-full stage optional.

## DEEP-WAR LEVER RANKING (2026-07-09 night corpus)

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

Bulk-collapse's theoretical floor: converting 3.17 M scattered reads into ~10² region-sized
reads (~60 MB/frame) costs single-digit ms of memcpy + parse; whether parse can be made
cheap enough (SIMD/vectorized row extraction) decides if full-walk alone reaches <20 ms.
Measure after Phase 2 — if deep-war is still >20 ms:

- **Engine dirty/version signal RE** (the gating unknown; no read-only OS dirty-page
  shortcut exists per `SUB50_REPORT §1.5`). A skip is legal only for provably-unchanged
  objects. Validation: long shadow soaks where change-driven and full-walk paths run
  side-by-side and must fold identical for the entire session, plus save-oracle checks,
  before it may ship (G1 at soak scale). If no sound signal is found, the fallback is
  deeper bulk-rewrite of the remaining walks + frozen-lane waivers.

Final target (G2/G3): deep-war warm avg **<20 ms** (internal 12–15 ms) speed-5 zero-skip on
the dev rig; hourly gate holds at 6 cores.

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
