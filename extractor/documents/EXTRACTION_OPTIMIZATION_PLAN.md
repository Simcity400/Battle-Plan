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

## Phase 0 — Ground truth (small)

1. One clean speed-5 zero-skip run at current head, deep-war save, dev rig → the honest
   pre-plan baseline (none exists today; latest smoke was cold/paused and gate-invalid).
2. Same run affinity-limited to 6 cores → portability baseline.
3. Wire the fold-equivalence harness (decode stream → fold → hash per hour) so G1 is a
   one-command check for every later phase.

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

## Phase 2 — The Big Five: bulk-read collapse (time) + delta emission (bytes)

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

## Phase 3 — Container & codec overhaul (bytes)

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
