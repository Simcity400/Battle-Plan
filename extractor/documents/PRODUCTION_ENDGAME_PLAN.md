# PRODUCTION ENDGAME PLAN — plan of record v2 (2026-07-11)

USER DIRECTIVE: full plan of attack, ready to execute the moment Codex is back. Maximum
parallelism. End state = PRODUCTION: capture times optimized, replay sizes optimized,
nothing missed, everything VERIFIED (retroactive-audit bar: census/coverage/data-closure,
adversarially gated, live-validated — "finished" means finished + verified, per the standing
scope ruling and audit mandate).

**v2 PROVENANCE:** v1 (01b290bc7) received its own adversarial Codex review per user
directive (tid 019f4f5c-c127-7a90-a995-1c4c97dcc571, verdict **AMENDMENTS REQUIRED**,
12 findings: 6 CRITICAL / 6 HIGH, zero rejected by orchestrator verification). All 12 are
folded in below; the review's "could-not-break" list (O8 design core, O6 columnar NULL,
d47 upsert deferral, R1/R2 direction, one-fix-round cap, live-game serialization awareness)
stands unchanged.

Standing laws bind throughout: zero silent data loss (zero-silent-gap accounting + bounded
skew), never mask failure bits, weaker-PC CPU budget, measure-first (no lever ships on a
model), one Codex fix round per review then orchestrator fixes, resumability law on every
Codex call, ≤15-min live turnarounds, `replay_interface/` untouched.

---

## 0. Where we stand (evidence, not vibes)

DONE + LIVE-VERIFIED (Codex re-stamp still owed where noted):
- **Bytes: ADVANCED — byte-CLOSURE GATE OWED** (was overclaimed "COMPLETE" in v1; review
  finding 8). 163–173x production compression. Window-27 LDM shipped; container levels
  exhausted; domain grouping = artifact; d53 *capture-time* delta CPU-negative; columnar
  measured NULL (−2.9%, `scripts/d53_columnar_probe.py`). Remaining UNMEASURED offline
  levers enumerated in Wave B — "probably dominated by LDM" is a model, not a measured NULL.
- **d47 RESTORED** (831,039 rows/509h vs all-time 0; parse errors 0; dead_roots 1) via
  root-refresh machinery + duplicate-char dedup. GATE OWED.
- **d69 PARTIALLY restored → SOUNDNESS-REVERTED 2026-07-11** (re-gate 019f5066 finding 1).
  The vacant/tombstone MEMBER-word classification stands (adjudicated sound twice). But the
  SLOT-level stale-skip (non-null slot with a stable non-group vtable = "beyond active
  extent") was UNSOUND: a mid-construction live group briefly holds a base-class vptr that
  reads as a stable in-module mismatch, so skipping it could silently drop a live group
  under full-replacement fold. Now FAILS CLOSED → d69 is accounted-PARTIAL on saves whose
  unused slots hold dangling non-null pointers (the observed 3-group save), contributing
  nothing to the fold there — a lawful accounted gap, not a silent wrong snapshot.
  **Populated-AND-sound d69 needs the manager's active-group COUNT offset (fixed-9-slot
  model is single-fixture + now doubly-refuted); that RE is BLOCKED on a live oracle and is
  the TOP d69 priority when the game is available.** `d69_stale_vtable_rva_sample` telemeters
  the reusing type for that RE. GATE for the fix round: e908a0f61 (self-verified; F5 covers).
- **d43** cold/empty semantics hardened (2 gate rounds, SOUND); sweep-completeness law
  (`coverage_complete()`) protects all adopting consumers; sweep enumeration holes fixed.
- **Modset binding (O5/M2)** live-validated (`resolved=true`, 20/24 DLCs). GATE OWED.
- **O4/U1** RTTI re-derivation self-proven 51/51. **O7** war-segment accounting bit shipped.
- **Perf shipped:** SEH read+region cache (3.57x), W0 (−43% d53 warm), prefetch relation=8,
  reader_concurrency=12, deferred fold, zstd-at-boundary, hour-latch (default OFF, gated).
- **Serial-wall lever IMPLEMENTED** (1cac48222): 10 post-join readers moved into the worker
  scope. The v1 claim "byte-neutral by construction" is WITHDRAWN (review finding 1):
  `thread::scope` proves borrow-safety, not temporal/observational equivalence on live,
  advancing memory. Live A/B so far: main-wall 46.4→0.15ms; NEW beat OLD −29% at adjacent
  states (68.7 vs ~97ms avg, p95 84.9 first-ever sub-100) and the apparent NEW blow-up at
  deep war (149.9ms avg) was game-load — OLD at deeper state was WORSE (220.0ms avg,
  p95 370.8). Deep-state NEW window (B3) + equivalence evidence still owed (P1a).

OPEN LEVERS / DEBTS (this plan closes every one):
perf: P1a scheduler decision, candidates prefetch, hour-latch default decision, pace
exogeneity, speed-5 re-acceptance with the FULL numerical acceptance set. correctness:
C0 domain/segment census, O8 SourceDisposition (with the false-green amendments), d43
elapsed-0 warmup, d47/d69 deep-war soak. bytes: Wave B closure gate. robustness: M1
modset-resolved catalogs, vanilla-tuned validation bounds audit, cross-modset matrix
(BLOCKS the production tag), U-series post-patch oracles. production: traceability matrix,
retroactive audit reconciliation, dress rehearsal with frozen requirements + automated
assertions, integrated release-candidate net gate, runbook.

---

## 1. WAVE G — the gate storm (Mode A read-only; G1/G2/G4/G5 parallel, G3 HELD for P1a)

Resumability law on each; thread ids to `~/.claude/codex_threads.log`; usage-limit deaths
queue + resume. **Gate/merge topology rules (review finding 9):**
- Every gate records the exact tree SHA it reviewed, in the dispatch queue next to its tid.
- Only the initial read-only reviews run in parallel. Fix/merge work serializes through a
  shared-file ownership queue (one lane owns `collector/src/lib.rs` at a time).
- Any gate whose reviewed files or prerequisites changed after its verdict gets rebased and
  re-run before its verdict is used for master advance.
- G5's reconciliation list regenerates after ALL fix commits land, not before.
- A **final integrated release-candidate net gate (F5)** reviews the exact tagged tree —
  a union of per-gate verdicts from different trees is NOT a release verdict.

| # | Gate | Scope | Verdict unblocks |
|---|------|-------|------------------|
| G1 | **Final net gate** (queued, prompt amended in CODEX_DISPATCH_QUEUE.md; resume tid recorded) | 8e10e70ff + 28f2043e1 + d5bb02247 + ebdc5bc4b + 1fbd9f3a4 + 2a9dc5091: sweep completeness, d69 stale/vacant/tombstone, d47 char-dedup key + degrade rule, revert completeness, O7 bit | d47+d69 verified-closed; master advance |
| G2 | **Modset gate** (queued) | e9f21a1c8: fail-closed resolution, ambiguity, exe-hash race, census key format, FFI | M-series proceeds on a verified base |
| G3 | **Serial-wall gate — DISPATCHED ONLY AFTER P1a completes** (finding 2), with the full A/B evidence + equivalence artifacts in the prompt | 1cac48222: no-alias argument, gate-token starvation (12-cap vs +10 workers), join ordering, panic semantics, lane_wall telemetry meaning shift, temporal-consistency/bounded-skew audit of country_bundle→d47, interior-mutability/shared-cache interactions | Perf endgame ships on a verified base |
| G4 | **O8 design review** (design in CONFIRMED_COMPLETE_DESIGN.md §2–5) | SourceDisposition types, ExpectedAbsent declaration rules, prerequisite plan, forbidden list; adversary attacks the tolerance semantics AND the C1 amendments in §3 below | Wave C implementation starts |
| G5 | **Audit-completeness gate** | AUDIT_LEDGER.md vs merged-work list since ff1d37bed: name every merged change lacking its audit row | Wave F reconciliation list |

Fix rounds: one per gate (Codex), then orchestrator fixes directly. Lossless-critical
verdicts get the standing extra final gate.

**Sandbox contingency (proven necessary):** the Codex read-only sandbox shell is broken on
this host (CreateProcessWithLogonW: 2). Every gate prompt inlines its full evidence (diffs
via `git show`, docs, manifests) so Codex needs zero tool calls; resume-by-tid carries
context across follow-ups.

## 2. WAVE P — perf endgame (live windows serialize on the ONE game; analysis parallel)

**MEASUREMENT REGIME (user directive 2026-07-11): all optimization A/Bs run at GAME SPEED 2**
(the user keeps the game at speed 2; speed 5 takes too long to run often). Relative verdicts
at speed 2 decide every lever. The SINGLE true-speed-5 acceptance run (P5) happens ONCE, at
the END, only after every lever is shipped / measured-null / declined — it is verification,
not iteration. Speed-2 absolute numbers are never quoted as production numbers.

Order chosen so each window answers one question; ~15 min each; gate-bracket every run
(pause/unpause = discard). DLL-hold gotcha: -SkipBuild between same-binary windows, 35s+
spacing (build-retry loop against the hold), NEVER cargo-build while a capture is in flight.

- P1a **Serial-wall scheduler decision — CLOSED 2026-07-11: SHIP NEW** (pending P1b + G3).
  ⚠ **SPEED PROVENANCE (user disclosure 2026-07-11): all recent live sessions ran at game
  speed 2, not speed 5** (user avoided restarts). Consequences: every OLD-vs-NEW comparison
  below is internally valid (both arms same speed, same session) so the SHIP NEW decision
  STANDS on relative evidence; but ALL absolute frame times in this table are SPEED-2
  numbers — the "p95 84.9 first sub-100" is NOT a speed-5 milestone and is hereby
  downgraded to a speed-2 observation. Speed 5 raises the game's own tick-thread CPU load
  (more contention — exactly the axis the NEW scheduler is sensitive to), so P5 acceptance
  MUST re-measure everything at true speed 5, and the G3 evidence pack must carry this
  caveat. The same annotation applies to the P2 candidates palindrome (relative verdict
  usable; effect size to be re-checked at speed 5 if it decides a default).
  Six counterbalanced 240s windows on the advancing live game (time order = load order):
  | window | code | ft_avg | ft_p95 | ft_max | crit path (ms/frame) |
  |---|---|---|---|---|---|
  | A1 | OLD | 98.0 | 119.0 | 154 | d53 38.7 |
  | A2 | OLD | 96.8 | 115.4 | 142 | d53 39.5 |
  | B1 | NEW | 68.7 | 84.9 | 117 | d47 35.4 |
  | B2 | NEW | 149.9 | 313.1 | 573 | d200 42.3 + d47 46.4 |
  | A3 | OLD | 220.0 | 370.8 | 540 | d200 49.2 |
  | B3 | NEW | 257.8 | 506.1 | 1210 | d200 75.1 + d47 63.3 |
  Basis: NEW −29% at matched moderate load (B1 vs A1/A2, later = disadvantaged position;
  first-ever sub-100 p95); main-wall 46.4→~0.2ms as designed; deep-war blow-up is GAME
  LOAD (adjacent-window load growth B2→A3 = +47% with a code change that should have
  HELPED OLD; B3's +17% over A3 is below that trend), and OLD fails the bar at depth at
  least as badly. Caveats carried forward: (a) weaker-PC/6-core window still QUEUED —
  release-note caveat until run; if NEW ever shows spare-core-only wins → redesign as a
  bounded dependency DAG; (b) deep-war measured crit path = country_bundle (75ms/frame at
  depth!) + d47 chain — this is the P5-miss optimization target, NOT d53; (c) d47 dead_roots
  scale with war churn (2–11/window) — invalidation-rebuild cadence is the d47 depth cost.
- P1b **Scheduling-equivalence evidence** (finding 1; feeds G3): paused-state OLD/NEW
  window pair for non-sweep domains (paused = cold baseline frame; sweep consumers are
  SOURCE_UNAVAILABLE by design, so d43/d47 equivalence rides the dataflow argument
  instead) comparing per-domain data hashes, row counts, status/failure bits, folded
  world hash; documented dataflow + bounded-skew statement for country_bundle→d47 (d47
  consumes the SAME frame's join output; its heap reads move EARLIER in the frame, not
  later; all live-memory readers carry intra-frame skew under both schedulers — the law
  is accounting, not atomicity); interior-mutability audit of the moved readers' shared
  caches. No "byte-neutral" claim anywhere until this passes.
- P2 **Candidates palindrome — CLOSED 2026-07-11: MEASURED NULL** (clean 6-window
  palindrome [32,16,0,0,16,32] at speed 2, p2cand_*_152732, applied distance verified
  per-window, mixed=false all six). Paired means: d53 read_time 41.9/41.5/42.7 ms and
  relations phase 36.1/35.4/35.9 ms for dist 0/16/32 — ~3% spread, inside single-window
  noise (same-arm windows differ more). `PREFETCH_DIST_CANDIDATES` stays default-off;
  entry goes in the known-NULLs table. (Relation-8 default from the earlier palindromes
  is unaffected.)
- P6 **M-series: materialise levers (three-phase-probe-directed; NEW 2026-07-11)**. The
  Debug three-phase window (threephase_deepwar_p1, 372 frames, speed 2) split the top lanes
  into guarded-read vs materialise (pure local CPU — the weaker-PC law's favorite target):
  | domain | lane ms | guarded | materialise |
  |---|---|---|---|
  | d53 diplomacy | 49.6 | 39.6 (80%) | 10.1 |
  | d29 ships | 40.8 | 35.3 (86%) | 5.6 |
  | d44 supply | 40.2 | 14.6 (36%) | **25.6** |
  | d28 oob_command | 39.3 | 15.6 (40%) | **23.7** |
  | d23 map_detail | 32.7 | 1.1 (3%) | **31.6** |
  (d47's attribution in this window is polluted by prewarm scope — re-measure; d52 shows
  the sampling error bar ~±10%.) Read-bound lanes (d53/d29) get acquisition levers only;
  the ~80ms/frame of materialise across d23/d44/d28 is CPU-side and byte-neutral to fix.
  - M1 **building-levels cache Mutex+clone → RwLock+Arc** — SHIPPED 97a5d902c (suite
    353/0); gate queued. Attacks d23 (97% materialise; multi-threaded province sampling
    serialized on one Mutex + ~26k Vec clones/frame with d44). **LIVE VERDICT BLOCKED ON
    GAME RELAUNCH (2026-07-11 ~17:10):** ABBA round CONFOUNDED by a war flare-up (canary
    d28, untouched by M1, rose +18.5%; building domains d23/d44/d48 inflate together with
    construction/damage churn; A1 vs A2 disagreed 24% on d23). Interleaved rerun with
    prebuilt DLL swaps (rename-aside law: in-place cp over a mapped DLL fails indefinitely;
    rename-aside + copy works) got 1 OLD window, then the GAME EXITED mid-w2 (NEW) — zero
    valid NEW windows. Note: NEW previously survived 2 full ABBA windows, so a crash link
    is unlikely but unexcluded. NEXT GAME BOOT: rerun m1_interleaved_ab2.sh
    (~/.claude/hoi4_gate_prompts/, stashed DLLs verified old=b0a9cd6a new=66987ec6),
    6-8 windows, judge on adjacent pairs with d28 as load canary.
    **RERUN 2026-07-11 23:14 — SAME OUTCOME (2nd time):** w1_old captured 409 frames
    (map_detail read_time avg 17.4ms), then the game EXITED before w2_new injected
    (w2 = ConnectNamedPipe timeout 244s; w3–w8 all fired against a dead PID, no manifests).
    So the game has now died right after w1_old on BOTH interleaved rounds (170151: w1_old
    372 frames then dead; 231437: w1_old 409 frames then dead). The crash lands in the
    ~40s post-w1 gap (w1 completes a full 240s + writes its manifest, so the game is alive
    THROUGH the old-DLL window). Candidates for the repeat: (a) game-side instability at this
    save's war depth independent of us; (b) something in the first post-window rename-aside
    swap; (c) user closed it. NOT yet attributable to NEW (w2 never landed a frame). The
    single valid window ran the pre-retirement OLD DLL, so its d69 (stale_slots=2449,
    flags=0) just re-confirms the silent-drop that e908a0f61 closed — not tip behavior.
    NEXT BOOT: run a LEADING-NEW order (new,old,new,old…) so the first — most reliable —
    window exercises NEW, and add a 5s post-w1 game-liveness probe to the driver to
    disambiguate (b) from (a)/(c) before blaming the swap.
    **RERUN 3 2026-07-12 00:06 — ROOT CAUSE ISOLATED (leading-NEW + liveness probes):**
    w1_NEW captured 423 frames CLEAN, post-w1 liveness = ALIVE; w2_OLD then hit
    ConnectNamedPipe timeout 244s and post-w2 liveness = GONE. So across THREE rounds the
    game dies in the gap before the SECOND window's injection EVERY time, and w1 survived as
    OLD (rounds 1-2) and as NEW (round 3) — **the DLL arm is EXONERATED; the trigger is the
    2nd injection/DLL-swap cycle within one game process, not NEW's code.** Prime suspect:
    the rename-aside swap reuses ONE shared DLL path (collector.dll) — w1 registers a
    WH_GETMESSAGE hook against that path; renaming a DIFFERENT dll onto the same path while
    the game still holds/resolves w1's hook is the classic crash (game loads a module that
    doesn't match the hook registration). HARNESS FIX (offline, staged): give each window a
    UNIQUE dll path (collector_w<N>.dll) so no live-hooked path is ever swapped under the
    game; also let w1's hook fully tear down before re-inject. This blocks ANY multi-window-
    per-session live experiment (P4 palindrome = 6 injections, P5) — not just M1 — so the
    fix is on the wave critical path. Single-window-per-launch experiments (tip-val, P3) are
    SAFE (first injection always survives). **M1 DISPOSITION: shipped-with-harness-caveat.**
    Best available evidence = tonight's two first-window captures (w1_new 000620 vs w1_old
    231437, CROSS-SESSION so load-confounded), canary-normalized by the M1-untouched d28
    (oob_command, which moved 27.37→22.87ms = 0.835× between sessions = pure load):
    | domain | old ms | new ms | new/old | ÷canary | read |
    |---|---|---|---|---|---|
    | d28 canary | 27.37 | 22.87 | 0.835 | — | load normalizer |
    | d23 map_detail (M1 PRIMARY) | 17.38 | 14.46 | 0.832 | **0.996** | NULL vs load |
    | d44 supply | 23.20 | 17.18 | 0.741 | 0.887 | −11% beyond load |
    | d48 construction | 11.37 | 9.14 | 0.804 | 0.962 | −4% beyond load |
    M1's headline claim (attack d23's 97%-materialise province sampling) is NOT supported —
    d23 tracks the canary exactly. Two secondary building domains show modest normalized
    wins. Single cross-session pair = directional only (±~10% canary noise). M1 the CHANGE is
    sound (gate SOUND, suite 353/0, non-regressing) so it STAYS shipped, but its perf value is
    smaller than pitched and the clean same-session verdict remains BLOCKED on the harness fix.
  - M2 candidate: d48 clones its whole `last_buildings` BTreeMap per frame (lib.rs
    ~23495) — measure share first.
  - M3 candidates: d44/d28 residual materialise after M1 (row building, delta-tracker
    lookups) — re-run the three-phase window post-M1 to re-rank before building anything.
  - Also seen in the same window: d53 relations phase = 56.1 of 62.9ms at deep war (89%)
    — the read-bound monster; W1.1 outcome-split timing is its probe (read-count≠time
    law applies: only a live paired A/B ships it).
- P3 **Hour-latch decision run**: one 600s window `--hour-latch` on the P1a binary — latch
  invariant + frame-cost check (ABBA already green; this is the post-serial-wall re-check).
  Then USER CALL on default-ON. 6-core portability run stays queued for the other rig.
- P4 **Pace-exogeneity experiment** (decides whether frame-time cuts speed the GAME up):
  fixed save+speed windows with domain subsets ~40/~80/~151ms; compare hour-interval
  distributions. Uses the subset presets; driver to write. (~45 min, can run overnight)
  **DRIVER READY 2026-07-11 (~/.claude/hoi4_gate_prompts/p4_pace_windows.sh + p4_analyze.py):**
  no code change was needed — the subset mechanism already exists end-to-end
  (`--scheduled-domain <name>` CLI, repeatable → control-file `scheduled_domains` (";"-joined)
  → collector `scheduled_domain_enabled` at lib.rs ~3480: EMPTY = all domains, non-empty =
  listed only, invalid = NONE i.e. fail-closed; host computes expected_domain_ids from the
  same list so a subset window gates against its own subset). Tiers from MEASURED w1-231437
  per-domain read_time_us: FULL = all 55 (297 lane-ms/frame), MID = minus top-5 heavies
  d53/d29/d28/d44/d23 (~166), LIGHT = 6-domain cheap tail (~6). Palindrome
  [full,mid,light,light,mid,full] × 240s at 1ms sampling — hour-boundary resolution is
  cadence-bound (frame loop reads the hour counter regardless of domains), so LIGHT keeps
  full resolution at ~50x less lane cost. Decision metric: game-hours/wall-second per tier.
  DEVIATION from the fixed-save design: runs on the advancing live game with palindrome
  ordering as the load control (a save reload needs the user); escalate to fixed-save arms
  only if the palindrome verdict is ambiguous.
- P5 **Speed-5 re-acceptance** (the benchmark of record, after P1a–P3 land). Full
  production preset, 600s runs at THREE strata: early-game, ordinary-war, deepest-war.
  **Numerical acceptance set (finding 4 — p95<100 alone is NOT the bar):**
  - accounting identity: observed == attempts + coalesced + in_flight, zero in-flight
    after flush (zero-silent-gap law);
  - coalesced-hour rate and failed-attempt rate, each ≤ a predeclared bound per stratum
    (bounds proposed from the P5 data, USER-approved before the tag);
  - max consecutive coalesces and max/percentile capture skew, bounded;
  - frame p50/p95/p99/max reported per stratum (p95 < 100ms target on THIS rig's strata,
    weaker-PC caveat listed);
  - CPU utilization + ms/game-hour pace.
  These numbers go in the README as the production spec.

Perf exit statement after P5: every known lever is shipped, measured-null, or explicitly
declined with evidence. The v1 claim "frame time is then critical-path-bound on d53" is
WITHDRAWN (finding 3 — under the new scheduler the deep-war crit path was
country_bundle→d47, not d53): P5 must report the MEASURED lane DAG + tail attribution per
stratum for the selected scheduler, and any follow-up optimization targets that measured
chain, not an assumed d53 floor. W1.1 outcome-split timing remains the d53 probe IF d53
owns the relevant tail.

## 2b. WAVE B — byte-closure gate (offline, fully parallel, no live game; finding 8)

Small, fold-identity-backed offline prototypes on existing captured streams. Predeclared
materiality threshold: a lever ships only if ≥3% total-artifact win at production settings
with decode/fold equality; below that it's recorded as a measured NULL in the known-NULLs
table. Each: record final .battleplan bytes, decode/fold equality, offline CPU + memory.
- B1 pack-time d53 explicit delta/tombstones (offline transform — the capture-time CPU
  verdict does NOT cover this).
- B2 trained zstd dictionaries at actual container-entry granularity (global + per-domain).
- B3 — **CLOSED 2026-07-11.** Final table (w6 corpus, 9.65GB decoded, w27+LDM unless
  noted): level 3 = 59.02MB (shipped) / 12 = 49.68 (−15.8%) / **19 = 42.37 (−28.2%,
  pack 57 MB/s single-core-idle ≈ 12 min per 40GB session — SHIP CANDIDATE:
  ReplayZstdLevel default 3→19)** / 22@w30 = 40.32 (−31.7%) **DECLINED** (only −4.8% more
  than 19 while window-30 needs ~1GB decode memory — violates the weaker-PC replay bound;
  window stays 27). **SHIPPED a09c772db (2026-07-11): DEFAULT_ZSTD_COMPRESSION_LEVEL 13→19**
  (production default; explicit -ReplayZstdLevel 3 A/B path unchanged; window 27 untouched).
  Lossless proof = new test level19_window27_roundtrip_is_byte_lossless (packs an
  LDM-exercising mixed payload via the production entry point at explicit {19,27}: streaming
  verify under the production window cap + sha256, entry method asserted ZSTD not STORE,
  byte-identity through the production Container reader). Suites snapshot 21/0,
  recorder_host 190/0+193/0. Codex Mode-A adversarial gate tid 019f516d: SHIP, zero defects;
  its residual = the launcher turnaround projection does not model container pack time in the
  skip-host-fold path (verified PRE-EXISTING and level-independent: projectedFoldSeconds=0
  whenever -RunHostFold is absent, launcher line ~229) — close with a real full-length
  finalization measurement at P5, then decide if pack-rate belongs in the projection.
  NOTE the old "level 3 = shipped" in the table above described the A/B launcher default,
  which persists by explicit flag; the CODE default 13 (used by production FullGame runs) is
  what moved to 19, i.e. production sessions gain −14.7% vs their previous actual size.
  Superseded partial note below:
- B3 (history) production-parameter zstd ultra/window/LDM sweep — **PARTIAL RESULT
  2026-07-11 (w6_d16 corpus, 9.65GB decoded, identical w27+LDM both sides): level 12 =
  49.68MB (−15.8%), level 19 = 42.37MB (−28.2%) vs production level-3 59.02MB.** Far past
  the 3% materiality bar — v1's "container levels exhausted" was WRONG (the old level
  bracket ran at default window; at w27 the level matters a lot). Columnar re-confirmed
  NULL at every level (−1.0%/+0.2%). SHIP CANDIDATE: deferred-repack level 3→19 (pack is
  offline post-session; decode speed/memory level-independent at fixed window). OWED
  before shipping: (a) pack-time cost measured on a weaker-PC-realistic budget (level 19
  on multi-GB corpora is minutes — document, maybe make it a launcher knob), (b) the
  level-22/w30 arm (killed mid-run to protect the user's live play session), (c) fold
  identity on the repacked artifact, (d) Codex gate on the repack-level change.
- B4 schema-level field elision where a decoder can reconstruct exactly (proof required).
- B5 any varint/delta representation actually measured, not dismissed by inference.
EXIT: "bytes CLOSED" = every row in this list has a measured number attached.

## 3. WAVE C — correctness endgame (implementation lanes; parallel with Wave P analysis)

- C0 **Domain closure census — BLOCKS C1 stage 2** (finding 6): machine-generated census
  from representative live manifests × the compact_contract roster (so no domain/segment
  escapes): per domain AND scheduled subsegment — per-hour rows, each individual
  status/failure bit's incidence, fold acceptance/rejection, first/last occurrence,
  consecutive-failure spans. EXIT: every recurring nonzero bit attributed to a concrete
  source + policy; ZERO unexplained failure-bit incidence. (d43's one-subsegment-fails-
  while-another-emits-188k and d69's populated-but-fold-rejected histories are exactly what
  this catches; d53 tolerated-partial history gets explicit attribution.) O8 must not
  classify flags nobody has attributed.
- C1 **O8 SourceDisposition** (XL; starts after G4 + C0; orchestrator implements with
  Codex gating each stage — Mode C sandbox is broken). **Amended semantics (finding 5):**
  - Disposition set per scheduled segment is EXACTLY: `Captured`, `MeasuredEmpty`,
    `ExpectedAbsent` (pre-capture positively proven, session-static, epoch-bound,
    invalidated on reload), `Failed`.
  - `Failed` is ALWAYS release-blocking and NEVER satisfies any Done/green criterion.
  - `WithExpectedAbsence` stays visibly distinct from fully-captured green in the recorder
    summary, manifest, and gates — never collapsed into one "GREEN".
  - Sweep-cold, timeouts, missing CLI inputs, empty caches, failed reads, and incomplete
    enumeration can NEVER produce ExpectedAbsent (sweep-cold is dynamic failure; it was
    wrongly listed as a disposition source in v1).
  - The forbidden list (d43/d47/d69 traps) ships as executable contract tests.
  - d17: production supplies no war bridge and a missing bridge is NOT ExpectedAbsent. If
    d53 is the authoritative war source, d17 gets an explicit `DerivedFrom(d53)`
    disposition with an equivalence proof; otherwise d17 is captured natively or removed
    from the scheduled roster by explicit product decision. No third option.
  - Data appearing after an absence declaration = capture it AND fail the run (design law).
  - Stages: 1 compact_contract types + wire format (versioned; never touches failure
    bits); 2 collector per-segment dispositions (informed by C0); 3 recorder gate: hourly
    requirement consumes dispositions — SOURCE from a declared ExpectedAbsent segment no
    longer fails the run, everything else unchanged; 4 fold/decoder passthrough + tests +
    G1-chain fold-identity proof.
  EXIT: a clean vanilla production run ends `Full` or `WithExpectedAbsence` (distinct),
  with today's chronic problems resolved to TYPED, ATTRIBUTED dispositions.
- C2 **d43 elapsed-0 warmup preflight** (S): needle warm-up before the first frame with
  bounded timeout (fail closed) — kills the last cosmetic SOURCE paint. After C1 lands it
  may be unnecessary (disposition covers it) — decide then, don't double-build.
- C3 **d47/d69 deep-war soak** (live, 1h+ window piggybacked on P4/P5 captures): d47 rows
  every hour, dead_roots ≈ churn, parse errors 0; d69 status 0 throughout; refresh/hold
  telemetry sane. Also re-check PARTIAL-upsert pricing from the same manifest (stays
  deferred unless churn-loss rate materially exceeds ~1 frame/session).

## 4. WAVE R — robustness endgame (fully parallel with C; different files)

- R0 **Traceability matrix — before C1/R1 schemas stabilize** (finding 7): bidirectional
  scope matrix for the standing everything-in-game ruling: in-game surface/panel →
  required dynamic values + cadence → extractor domain/schema supplying them | static
  game/mod asset | replay-derived (with derivation proof) → owning replay consumer +
  acceptance status → explicit BLOCKER row where no source exists. Battle-plan arrows,
  particle triggers, all map modes, every UI panel field get rows. A missing extractor
  field is irreversible after the live game ends — this matrix is what "nothing missed"
  MEANS. F2 validates the artifact against it mechanically; F4 gets it ACCEPTED by the
  replay layer.
- R1 **M1 modset-resolved catalogs** (M): static catalogs (templates/equipment, loc join,
  decisions, ideas, ships, gfx) currently generate from VANILLA files unconditionally.
  Make generation take the recording's modset (manifest.modset from M2) and resolve
  game-file reads through the mod overlay order; catalog manifests record the modset they
  were built against; recorder REFUSES (fail-closed warning) a catalog whose modset differs
  from the capture's. Codex gate on the resolver.
- R2 **Validation-bounds audit** (M): sweep every hardcoded plausibility bound tuned on
  vanilla observation (d47 char_id ceiling, skill≤15, vector caps, tag_db_id≤2048,
  MAX_MEMBERS_PER_GROUP=512...) into one documented table (constant, value, evidence,
  failure mode when exceeded, per-bound behavior mod-safe? fail-closed-with-attribution?).
  Every bound must fail closed WITH its attribution sample (the d69 pattern) so a modded
  save that exceeds one names itself in a single capture. Mostly mechanical; Codex can run
  the sweep as a Mode-A survey while orchestrator wires the samples.
- R3 **Cross-modset matrix (USER-DEPENDENT — BLOCKS THE PRODUCTION TAG; finding 11)**:
  one session each on (a) a small cosmetic mod, (b) a DLC-disabled combo, (c) a major
  overhaul (Kaiserreich-class). Per session: modset resolves in manifest; per-domain
  rows/status vs vanilla baseline; R2's bounds report zero unexplained trips; d53
  offsets-vs-scan shadow referee green. **Release policy (explicit, truthful):** the
  standing mod-support mandate makes mod robustness a product requirement, so the
  production tag WAITS on R3. If the user chooses to ship earlier, the release is labeled
  "vanilla/current-build production; mod support provisional" — that relabeling is a USER
  decision, not a default. Game-version robustness is likewise stated as patch-day
  fail-closed PREPAREDNESS (R4), not proof over unseen versions; the first post-patch
  version must pass R4 + a capture/shadow matrix before being declared compatible.
- R4 **U-series remainder** (S, after O4): patch-day runbook — rtti_rederive --rederive,
  d53 shadow-mode referee capture, sentinel checks, bounds re-validation — written as a
  single checklist doc with commands. (The vtable half is already self-proven.)

## 5. WAVE F — production finalization (after G1–G5, P5, B-wave, C0–C3, R0; R3 blocks the tag)

- F1 **Retroactive audit reconciliation**: close G5's REGENERATED list (post-fix-commits) —
  every merged change has its AUDIT_LEDGER row (gate verdict, live validation, citation
  check). The audit bar is the standing definition of verified; no exceptions ride into
  the release.
- F2 **Dress rehearsal** (the production acceptance run; finding 12): FREEZE the acceptance
  version of REPLAY_DATA_REQUIREMENTS.md (recorded SHA) BEFORE the run. Boot → full
  multi-hour session at speed 5 (deep war) → dispositions all Captured/MeasuredEmpty/
  ExpectedAbsent (C1) → deferred fold → .battleplan → **automated artifact assertions**
  (not spot-checks): domain/segment roster completeness, disposition legality, failure-bit
  incidence zero-unexplained, hour accounting + skew bounds, canonical fold hashes,
  decoder queries for EVERY required family in the frozen requirements + R0 matrix,
  static-catalog/modset binding. Any drift classifies as: implementation defect (fix +
  rerun) | deliberate scope change (explicit USER approval — never an incidental doc
  edit) | replay-layer responsibility (entered in R0 matrix with consumer acceptance).
  The requirements doc is NEVER edited to match a deficient artifact.
- F3 **Production runbook + release**: one doc — launcher invocation (preset, latch flag
  per P3 decision, modset expectations), the acceptance numbers from P5/F2, patch-day
  checklist (R4), known-NULLs list (now including Wave B rows), owed-maintenance list
  (incl. the 6-core portability caveat until run). Tag ONLY after F5.
- F4 **Handoff to the replay layer** (via EXTRACTION_INTERFACE_REQUESTS.md): stream spec
  version, disposition semantics (C1), modset manifest fields (M2), the R0 traceability
  matrix — and the handoff REQUIRES the replay layer's accept/reject on the contract
  (finding 7); a one-way informational note does not close it.
- F5 **Integrated release-candidate net gate** (finding 9): one final Mode-A adversarial
  gate over the exact tree being tagged (SHA in the prompt), covering everything merged
  since the last verified baseline as an integrated whole. The tag exists only after F5
  is SOUND.

## 6. Parallelism map + shared-resource locks

```
now ──┬─ G1 ─┬─ (fix rounds serialize on lib.rs ownership) ─→ master advance
      ├─ G2 ─┤
      ├─ G4 ──→ C0 census → C1 (staged impl; Codex gates each stage)
      ├─ G5 ──→ F1 (list regenerated after fixes)
      └─ B1–B5 offline byte probes (no game, no lib.rs — fully parallel)
P1a/P1b (game) → G3 dispatch → P2 → P3 → P4 → P5 → C3 soak → F2 rehearsal
orchestrator (between windows): R0 matrix, C2, R2 wiring, R4 doc, F3 drafting
user: P3 latch default call · P5 coalesce bounds approval · R3 modded sessions ·
      early-ship relabel decision (if ever) · credits
```

**Serial resources (finding 10 — live windows are NOT the only one):** the live PID +
injected DLL session, module-dir DLL/host deployment, `collector_control.json`, the
recorder pipe/host instance, capture run directories, release target dir, and the master
merge queue for hot files (`collector/src/lib.rs` above all). Protocol:
- ONE capture-session lock (lockfile in module_dir): capture drivers take it; any build or
  deploy script REFUSES while it exists (fail closed). No cargo build while it's held —
  and no HEAVY OFFLINE COMPUTE either (zstd sweeps, stream re-encodes, big folds): CPU
  contention during some-but-not-all windows biases the arms (near-miss 2026-07-11: a B3
  sweep launched mid-palindrome; killed before contamination — palindrome pairing would
  have partially saved it, but the rule is absolute).
- Control-file writes only by the lock holder; per-run control snapshot + DLL/host hash
  recorded in the run manifest (already partially true via manifest.dll).
- Lanes build in their own worktree with isolated CARGO_TARGET_DIR; deployment to
  module_dir happens ONLY from the orchestrator, between windows.
- Unique run directories (already true); master merges of hot files serialize through the
  ownership queue in §1.
Codex sandbox contingency: gates/surveys/design run with fully-inlined evidence; the
orchestrator implements; same plan, same order.

## 7. Definition of DONE (production)

1. Every commit since the last verified baseline carries a SOUND adversarial verdict
   (G1–G3, C1 stages, R1) — no unreviewed lossless-critical code in the release — AND the
   integrated F5 gate is SOUND on the exact tagged tree.
2. P5 acceptance per §2's full numerical set (accounting identity, bounded coalesce/skew
   rates with USER-approved bounds, stratified p50/p95/p99/max, CPU + pace) — not p95
   alone.
3. Bytes: Wave B closure — every candidate lever has a measured number (shipped, or
   measured-NULL in the known-NULLs table). No "probably dominated" entries.
4. A clean vanilla production run ends with every scheduled segment `Captured` /
   `MeasuredEmpty` / prevalidated `ExpectedAbsent` (visibly distinct from full green),
   and the dress rehearsal artifact (F2) passes the automated assertions against the
   FROZEN requirements + R0 matrix.
5. Every extraction domain and scheduled subsegment: rows flowing or a typed NON-FAILED
   disposition. `Failed` never satisfies Done. Zero unexplained failure-bit incidence
   (C0 census clean). d47, d69, d17 all closed (d17 via native capture or proven
   DerivedFrom(d53), by explicit decision).
6. Mod robustness EVIDENCED: R3 matrix PASSED (blocks the tag; only the user can relabel
   to vanilla-only provisional). Patch-day checklist self-proven where possible; version
   compatibility claimed only per-version after R4 passes.
7. AUDIT_LEDGER reconciled (F1 on the regenerated list); runbook + release tag exist
   (tag after F5 only).

## 8. Immediate actions (revised order)

1. ~~Health-probe~~ DONE (real xhigh job returned; sandbox shell broken → inline-evidence
   contingency active).
2. ~~Plan self-review~~ DONE (this v2 is the one fix round applied).
3. Finish P1a: land B3, publish the stratified A/B table + decision in this doc.
4. Dispatch G1, G2, G4, G5 in four parallel Mode-A calls with fully-inlined evidence,
   each prompt carrying its reviewed tree SHA. G3 waits for P1a/P1b.
5. Build P1b equivalence evidence (paused-state pair) in the next live window; draft C0
   census script + R0 matrix skeleton while gates run.
6. As verdicts land: fix rounds (one each, serialized on file ownership) → master advance
   → C1/R1 → Wave B probes overnight.
