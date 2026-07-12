# CODEX DISPATCH QUEUE — status of record (updated 2026-07-10 late session)

Orchestrator-owned. Method: `documents/LANE_WORKFLOW.md`. Context brief every lane must read:
`extractor/documents/CODEX_LANE_BRIEF.md`. Protocol: `C:\Users\Example\.claude\CODEX_PROTOCOL.md`.

| # | Job | Mode | Blocks | Status |
|---|---|---|---|---|
| 0a | Adversarial gate: `hour_latch` | A | — | **CLOSED**: 6 adversarial rounds, merged; live-validated twice (7290/7290 + 5004/5004 hours accounted, invariant held) |
| 0aF | hour_latch fix lane | C | — | **CLOSED** (merged with 0a) |
| 0b | Adversarial gate: `d53 delta_partial` | A | — | **CLOSED**: adjudicated contract-text (brief §3 carve-out); feature ships opt-in OFF. Its live A/B is **SUPERSEDED** — production zstd landed at 173x live makes the byte win redundant and the measured +46–68 ms CPU cost stands; do not enable, do not spend a live window on it |
| 1 | Three-phase timing instrumentation | — | — | **CLOSED**: merged; live split ~60% guarded acquisition / ~40% materialise |
| 1R2 | Re-review of Job-1 diff | A | — | **CLOSED** (ran, merged) |
| 2 | Acquire/process split (or its burial) | C | — | OPEN — fork input partially in: lane-wall walls now live in every production manifest (see below); ~60/40 split says BOTH paths viable; next concrete lever = prefetch live A/B (plumbing landed 1b8ed7d66) |
| 3 | Prefetch-dist control plumbing | — | — | IMPLEMENTED (orchestrator) 1b8ed7d66; suites green (collector 348/0, recorder_host 178+192/0, lane 114/0); Codex Mode-A gate DISPATCHED (thread in `~/.claude/codex_threads_hoi4.txt`) |
| 4 | Confirmed-complete / bridge expected-absence design | A | — | **DESIGN CLOSED 2026-07-10** — Codex analysis REFUTED the missing-bridge premise (verified: statuses decode SOURCE/PARTIAL not BRIDGE; d43=188k rows, d69=27.7k groups captured). Design of record: `CONFIRMED_COMPLETE_DESIGN.md` (typed SourceDisposition + auto prerequisite plan + tri-state latch completion; defects first). Spawns jobs 5–7 |
| 5 | **d47 emit defect** — ROOT-CAUSED: 96.6% stale root cache (prewarm early-returns forever; 1 dead cached character discards the whole frame) + latent politics-delta-as-full-snapshot | C | REAL live data loss | **ROOT-CAUSE DONE**; backstop LANDED 55287bd6a; **REPAIR READY** (gen-aware cache + rejection recovery + full-topology + evidence preservation — design in CONFIRMED_COMPLETE_DESIGN.md; needs Mode-C sandbox + live sweep-generation-cadence check) |
| 6 | d43/d69 attribution | A | — | **CLOSED 2026-07-10**: d43 NOT chronic (1 cold frame + latent None-vs-Some(empty) bug, small fixes staged); d69 chronic + UNTOLERABLE — fold rejects every frame, tech-sharing contributes NOTHING to replays today; one collapsed boolean blocks attribution. Fix plan in CONFIRMED_COMPLETE_DESIGN.md §d43/d69; spawns job 9 |
| 9 | **d69 reader fix** (per-reason counters first, then active-extent topology RE, shadow+oracle gate) + d43 small fixes (None-vs-Some(empty), preflight warmup) | C | d69 replay coverage = 0 today | READY |
| 7 | d17 war-relations segment coverage (silently absent without war_relation_bridge, unflagged) | C | zero-silent-gap | READY — anchored native reader or explicit segment accounting |
| 8 | SourceDisposition + prerequisite-plan implementation (design §2–5) | C multi-lane | truthful confirmed-complete | AFTER jobs 5–7 |

2026-07-10 lane-wall finding (orchestrator): per-domain critical-path walls (`lw_d*`) are
POPULATED in every advancing-game production manifest (`compact_stream.lane_wall`); the earlier
"empty lw_d" observation was a paused-game cold-frame artifact (paused = 1 cold frame ⇒
lane_wall.frames=1). Live numbers (latchsoak2, 3471 frames): critical concurrent lane avg
60.7 ms/frame, d53 owns 71%; PLUS a separate ~42 ms/frame SERIAL main-thread wall
(main_wall_us_total/frames) — that serial tail is a large un-mined lever, recorded for Job 2.

Mode A invocation (verbatim; per global rules — never drop the model/effort pins; RESUMABILITY
LAW: NEVER `--ephemeral`, always capture + persist the thread id):

```bash
cd "/c/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/mod/battle-plan"
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
printf '%s' "$PROMPT" | codex exec --json -s read-only -c approval_policy=never \
  --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=xhigh -o "$OUT" - > "$AJ"
TID=$(python -c "import json,sys;print(json.loads(open(sys.argv[1]).readline()).get('thread_id',''))" "$AJ")
# persist $TID durably (e.g. ~/.claude/codex_threads_hoi4.txt); verify exit 0 AND non-empty
# $OUT; retry once — EXCEPT usage-limit/out-of-credits (queue + resume later, never retry into)
```

The `cd` matters: the read-only sandbox roots at the invocation cwd, and gate jobs need
file access to the repo.

---

## Job 0a — Independent adversarial gate: `hour_latch` (Mode A)

Why owed: lossless-critical diff, merged inert (default OFF) on 2026-07-10; per
`CODEX_LANE_BRIEF.md` §7 the implementer's own review (which ran, returned REFUTED, one fix
round applied) does not clear it. **It may not be ENABLED until this gate runs clean.** See
plan-of-record §"OUTSTANDING INDEPENDENT GATES".

`$PROMPT` body:

```
You are an ADVERSARIAL REVIEWER (gate, not implementer). Your sandbox is read-only; output
findings only, do not attempt edits. You have no memory of this project — everything you need
is in this prompt plus the files named below, which you can and must read.

REPO: the cwd. Rust workspace at extractor/native/. Read first for context and vocabulary:
extractor/documents/CODEX_LANE_BRIEF.md (esp. hard requirements + losslessness vocabulary)
and documents/LANE_WORKFLOW.md ("prove don't assert").

SYSTEM UNDER REVIEW: the hour-latch feature, final merged state (review the CURRENT state of
the files, not a diff):
- extractor/native/collector/src/hour_latch.rs  (entire file; core is HourLatch::observe
  ~line 247: delta accounting over last+1..=raw, ~lines 260-276)
- extractor/native/collector/src/lib.rs ~line 1263 (runtime is Option<HourLatchRuntime>,
  constructed only when hour_latch_enabled && stream_format == Compact)
- extractor/native/collector/src/lib.rs ~line 2191 (control parse:
  parse_json_bool(.., "hour_latch_enabled").unwrap_or(false) — default OFF)
- its tests in the same files

CLAIMED GUARANTEES — your job is to REFUTE any of them. Attack, don't confirm:
1. OFF (default): behaviour byte-identical to pre-feature — no thread spawned, no extra
   game-memory reads, no stream records added or reordered, fold input unchanged.
2. ON: ZERO-SILENT-GAP ACCOUNTING — every game-hour is accounted exactly once as attempt or
   reason-tagged coalesce; invariant observed == attempts + coalesced + in_flight, with
   in_flight == 0 after flush. Silent loss is prevented by DELTA accounting (last+1..=raw),
   NOT by poll rate — correctness must hold for arbitrarily large jumps between polls.
3. The previously-found bug (a stale sweep read fabricated a false reload + a duplicate
   capture) is actually fixed in the current state, not merely narrowed.

ATTACK SURFACES to cover at minimum: i32 game_hour_raw edge cases (negative, i32::MIN/MAX,
wraparound of the i64 delta); backward jumps (save reload) and raw == last; the very first
observation (seeding vs counting); shutdown/flush ordering (can in_flight be nonzero after
flush, or an hour be double-counted across flush?); races between the sweep thread and the
latch (TOCTOU on last); counter overflow; whether any code path can drop an hour without a
reason tag; whether enabling it changes any byte of the data records themselves.

OUTPUT FORMAT (exactly):
VERDICT: SOUND | REFUTED
FINDINGS: numbered list; each = claim attacked, concrete failure scenario (inputs/state ->
wrong outcome) with file:line evidence, severity (SILENT-DATA-LOSS / CORRECTNESS / MINOR).
If SOUND: list the specific attacks you ran that failed to break it (so the gate is auditable).
Before reporting, re-read C:\Users\Example\.claude\CODEX_PROTOCOL.md and try to refute your own
findings once.
```

## Job 0b — Independent adversarial gate: `d53 delta_partial` (Mode A)

Same standing as 0a. The lane's own review found **two real silent-data-loss bugs** its fuzzer
missed (empty-first-frame baseline desync; mutate-before-validate in `apply_emit_mode`); this
gate verifies the fixed final state independently. **`delta_partial` may not be enabled until
clean.**

`$PROMPT` body:

```
You are an ADVERSARIAL REVIEWER (gate, not implementer). Read-only sandbox; findings only.
You have no project memory — read extractor/documents/CODEX_LANE_BRIEF.md (hard requirements,
losslessness vocabulary: baseline/delta/tombstone algebra, COMPACT_STATUS_* flags) and
documents/LANE_WORKFLOW.md first.

SYSTEM UNDER REVIEW: d53 (diplomacy) delta emission under partial frames — the opt-in
delta_partial mode, final merged state:
- extractor/native/collector/src/lib.rs ~line 1600 (D53EmitMode::parse, default Delta) and
  the d53 emit path: apply_emit_mode, baseline/delta/tombstone construction, tracker-advance
  rules. Grep anchors: "D53EmitMode", "delta_partial", "apply_emit_mode".
- extractor/native/recorder_host/src/world_fold.rs ~line 1431 (d53 hour overlays cleared each
  hour) and ~line 10530 (fold rejects DELTA|PARTIAL) — a ~58-line edit landed here.
- KEY CONTRACT FACT: COMPACT_STATUS_PARTIAL_DATA is a MEMBER of
  COMPACT_STATUS_FAILURE_MASK (extractor/native/compact_contract/src/lib.rs ~line 186).
  This exact fact caused a shipped bug once (an emit gate silently bypassed delta). Check
  every mask test in the paths above against it.

CLAIMED GUARANTEES — attack them:
1. Default mode (delta): byte-identical AND CPU-identical to pre-lane. The G1 fold chain hash
   over the deep-war corpus was reproduced exactly by the orchestrator despite the
   world_fold.rs edits — refute by finding any default-mode input whose fold output changes.
2. delta_partial ON: lossless under partial/failed frames. Specifically: trackers NEVER
   advance on a frame whose status intersects COMPACT_STATUS_FAILURE_MASK; a reader that
   folds baseline + deltas + tombstones reconstructs exactly the full-emit stream for every
   interleaving of {ok, partial, failed, empty} frames. ⚠ WORDING TRAP (caught by the Opus
   gate 2026-07-10): the invariant is FOLD-IDENTITY-WITH-FULL-EMISSION, and delta_partial
   *correctly* tombstones rows absent from a partial view — full emission drops torn rows for
   that hour too. Do NOT "harden" the emit path to suppress tombstones on partial frames to
   satisfy a "never from a partial view" reading; that would break byte-identity and silently
   retain torn rows the full-emit reference discards.
3. The two previously-found bugs are dead in the current state: (a) empty-first-frame
   baseline desync — last_emitted.is_empty() used as a seeded-sentinel; (b)
   mutate-before-validate in apply_emit_mode (state mutated before the mode/status check
   could reject).

ATTACK SURFACES at minimum: first frame empty vs absent vs failed; partial frame immediately
after baseline; two consecutive partials; entity disappears during a partial frame (may a
tombstone be emitted from incomplete evidence?); mode switched mid-stream; fold-side: does the
DELTA|PARTIAL rejection at world_fold.rs interact with the hourly overlay clearing to drop or
duplicate an hour's rows?

OUTPUT FORMAT: identical to the hour_latch gate — VERDICT: SOUND | REFUTED, numbered findings
with file:line evidence and severity, and if SOUND the list of failed attacks. Re-read
C:\Users\Example\.claude\CODEX_PROTOCOL.md and self-refute once before reporting.
```

**Gate handling (orchestrator):** REFUTED with SILENT-DATA-LOSS on either → the feature stays
un-enableable; one Codex fix round in a fresh Mode C lane, then I fix directly. SOUND → flip
the plan-of-record row to gate-passed; live gates (shadow-soak/A/B/6-core) remain before
enablement. Never claim a gate that did not run.

---

## Job 1 — Three-phase timing instrumentation (Mode C lane)

### Why (one paragraph of context)

The burst-correlation study (lane_reports/burst-correlation.md) reframed the target: zero-skip
needs a **~30–40 ms whole cycle** (not the mythical ≤25 ms), a ~3.8× cut from today's ~151 ms
sweep. One candidate lever is the acquire/process split: do only what MUST touch game memory at
the hour boundary, defer the rest. But d53's encode is already known-small (7.9 ms vs 49.3 ms
read), so **the split only pays if materialisation (building rows) is a large share of lane
wall — and nobody has measured that split.** The lane-wall counter measures per-domain wall;
`encode_append_span_us` measures frame-level encode; the gap — guarded acquisition vs
materialise *within* a lane — is unmeasured. Measure before building (LANE_WORKFLOW: a lane is
authorized to refute its own brief — if the numbers say the split is dead, that is a success).

### Design (the lane implements this, or refutes it with numbers)

Ride the existing read-counter probe machinery end-to-end — it was built for exactly this and
lands in the manifest with **zero host changes** (domain-250 DOMAIN_PERF diagnostic units,
name→sum aggregation):

- Frame phases already measured: `prewarm_total_us`, `worker_read_span_us`
  (collector/src/lib.rs:14878), per-lane walls via `FrameLaneTracker` (lib.rs:13916; span
  origin = worker_read_started so offsets compose), `encode_append_span_us` (lib.rs:13626 and
  16582), all carried in `CompactFramePhaseTiming` (lib.rs:4142) and the rc probe record
  (lib.rs:51042).
- Counter substrate: `rc_bump` gated on `RC_ENABLED` relaxed load (lib.rs:50980), thread-local
  `RcLocal` (lib.rs:50861), per-domain attribution via RAII `RcDomainScope` (lib.rs:50906,
  innermost-scope wins, out-of-range ids refuse to tag), per-frame drain
  `rc_domain_take_and_reset` (lib.rs:50943), leaked-name units `rc_d{id}_*` (lib.rs:50963).

**Add phase-1 time accumulation:** a new per-domain counter `RC_PD_GUARDED_US` (extend
`RC_PD_N` 4→5, add `rc_d{id}_guarded_us` to `rc_domain_unit_names`) plus the frame-level
`rc_seh_guarded_us` / `rc_rpm_guarded_us` twin in the frame counter set. Accumulate elapsed
time around the **logical guarded-read entry points** — scalar read (lib.rs:~51224), bulk read
(lib.rs:~51241 and ~51359) — into `RC_LOCAL`, routed per-domain by the existing tag. Inclusive
of cache-hit reads: the split boundary we care about is "code that walks game memory via the
read primitive" (not deferrable without the traversal) vs "row building over already-fetched
values" (the deferrable candidate). Derived, offline: per domain,
`materialise_us ≈ lane_wall_us − guarded_us`; phase 3 = `encode_append_span_us` (frame-level
is sufficient; it is known-small).

**Overhead discipline (hard requirements):**
- Gate OFF (production): byte-identical stream, and the per-read cost must remain EXACTLY the
  existing single `RC_ENABLED` relaxed-load early-return — the timing code sits behind it.
- Gate ON: the instrument must not lie about itself. Two `Instant::now()` (QPC) calls per
  logical read costs ~40–60 ns; at 10⁵–10⁶ reads/frame that is 4–60 ms — potentially bigger
  than the thing measured. The lane MUST measure reads/frame on the synthetic fixture first,
  then choose: direct timing if distortion <5% of lane wall, else **sampled timing** (time
  every Nth logical read, N = 16 or 64 power-of-two mask on the existing RC_READS count,
  scale by N) with the sampling factor recorded in the unit name or lane report. Report the
  measured self-cost either way; the final lane report must state the error bar.

### Acceptance gates (all must pass before merge; orchestrator re-runs them independently)

1. `cargo build --release` + full workspace suite from the lane worktree: 1214 passing / 0
   failed, allowing ONLY the known pre-existing flake
   `diplomacy_intel_operatives_live::payload_phase_times_populated_and_bounded_by_outer_build`
   (extractor.rs:6652; reproduced on clean base by two prior lanes — floors to 0 on a fast rig).
2. G1 fold identity on the retained deep-war 600 MiB prefix: chain hash
   `bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7` reproduced exactly
   (fold_check hash; exact invocation in CODEX_LANE_BRIEF.md gates). Read the stream IN PLACE —
   it has no backup; never copy or move it.
3. Gate-OFF byte identity: a fixture capture with `extraction_perf_gate=false` produces a
   byte-identical stream pre/post patch.
4. Gate-ON: new units appear in the manifest through the EXISTING name→sum path — zero
   recorder_host changes (that is the point of riding domain 250). Invariant test:
   per-domain `guarded_us ≤ lane_wall_us` on the fixture.
5. Unit tests: counter drain round-trip incl. the new index; RcDomainScope nesting attributes
   guarded time to the innermost tag; sampling scaler (if used) is exact on a deterministic
   fixture.

### Lane discipline

Worktree lane `lane/three-phase-timing` branched from CURRENT master (freshness-gate: verify
`git merge-base --is-ancestor master HEAD`; if the worktree is stale, reset --hard to master
first). Write the lane report to
`extractor/documents/lane_reports/three-phase-timing.md` — NOT the plan doc, NOT
`extractor/native/documents/` (a prior lane got this wrong). Do not touch: `replay_interface/`
(another agent owns it), `documents/AGENTS.md` (standing user rule), the plan of record.
Implementer review runs per LANE_WORKFLOW; the orchestrator's independent gate follows before
merge (this is NOT lossless-critical — no stream-content semantics change — so the standard
single gate suffices).

### Mode C dispatch (orchestrator runs; sandbox roots at cwd, so cd FIRST)

```bash
git -C "/c/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/mod/battle-plan" \
  worktree add ../hoi4_lane_three_phase_timing -b lane/three-phase-timing master
cd ../hoi4_lane_three_phase_timing
OUT=$(mktemp /tmp/cdx_XXXXXX.txt)
printf '%s' "$PROMPT" | codex exec --ephemeral -s workspace-write -c approval_policy=never \
  --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=xhigh -o "$OUT" - >/dev/null
```

`$PROMPT` body = the whole of this Job 1 section verbatim (Why + Design + Acceptance gates +
Lane discipline), prefixed with:

```
You are the IMPLEMENTER for lane/three-phase-timing, cwd = your worktree. You have no memory
of this project. Read, in order: extractor/documents/CODEX_LANE_BRIEF.md,
documents/LANE_WORKFLOW.md, extractor/documents/lane_reports/burst-correlation.md. Then
implement the design below — or refute it with measurements if the fixture numbers say the
overhead cannot be bounded; a numbers-backed refutation is a valid deliverable. Commit to the
lane branch only. codex is on the Bash PATH. Your final message must list: files touched,
gates run with verbatim results, and the lane-report path.
```

## Job 1R — Adversarial review of the Job-1 diff (Mode A) — BLOCKS the merge

Job 1 was implemented directly by the orchestrator during the Codex outage; the lane sits at
`lane/three-phase-timing` commit `d56e2bbe7` (worktree `..\hoi4_lane_three_phase_timing`),
one commit ahead of master, all gates green (check 0-new-warnings; suite 1216 passed / 1
known flake; G1 chain `bc517771…` exact; gate-OFF inertness test). It does NOT merge until
this review runs clean. Run Mode A with cwd = that worktree (read-only sandbox roots there).

`$PROMPT` body:

```
You are an ADVERSARIAL REVIEWER (gate, not implementer). Read-only; findings only. You have
no project memory — read extractor/documents/CODEX_LANE_BRIEF.md and the implementer's
self-report extractor/documents/lane_reports/three-phase-timing.md (skeptically). The diff
under review: `git show d56e2bbe7` in the cwd.

WHAT IT DOES: adds phase-1 "guarded acquisition" timing to the collector's read-counter
machinery (extractor/native/collector/src/lib.rs). Sampled timing (every 64th logical read,
free-running per-thread counter, NANOSECOND accumulation) over read_scalar/read_bytes/
read_into — whose rc_bump(RC_READS/RC_BULK_BYTES) calls were refactored into a combined
rc_read_probe()/rc_read_probe_end() pair — plus unsampled per-call timing over
read_bytes_scan. New slots RC_GUARDED_SPL_NS/RC_GUARDED_SPL_N/RC_SCAN_GUARDED_NS (RC_N
8→11), per-domain twins (RC_PD_N 4→7), names in RC_UNIT_NAMES_SEH/RPM + rc_domain_unit_names.
Estimator: guarded_ns ≈ spl_ns × (reads/spl_n) + scan_guarded_ns.

CLAIMED GUARANTEES — attack:
1. COUNTER SEMANTICS UNCHANGED: RC_READS/RC_BULK_BYTES increments identical to the old
   rc_bump pairs on every path (ordering vs len==0 early-returns included). Drift silently
   corrupts every A/B against historical manifests.
2. GATE OFF: byte-identical production behavior; per-read cost = one RC_ENABLED
   relaxed-load early-return; no timing, no counts, no stream change.
3. READ BEHAVIOR UNCHANGED: closure refactors preserve exact control flow and return
   values, incl. read_into's buf-unspecified-on-false contract; read_bytes_scan's probe
   covers ALL exits (a missed exit = silent undercount).
4. NAME/INDEX ALIGNMENT: the 3 new names sit at indices 8/9/10 in BOTH frame name arrays;
   rc_pd_index maps new RC_* to the right RC_PD_* slots. Off-by-one = every manifest unit
   mislabeled, nothing crashes.
5. SAMPLING SOUNDNESS: free-running spl_ctr (deliberately NOT drained by flush_indexed)
   cannot double-count/lose samples across RcDomainScope nesting; can a scope change occur
   between probe begin and end within one primitive call, and if so where does the time
   attribute; u64 wrap harmless.
6. TEST SOUNDNESS: the eventually() retry harness cannot mask a real determinism bug (a
   broken stride/routing must fail all 5 attempts); RC_TEST_LOCK serialization correct.
Also: any remaining production caller doing manual rc_bump(RC_READS,..) that now diverges;
probe pair straddling an early return anywhere; RcLocal Drop-flush with a probe in flight.

OUTPUT FORMAT: VERDICT: SOUND | REFUTED; numbered findings with file:line evidence and
severity (SILENT-DATA-LOSS / CORRECTNESS / MEASUREMENT-LIE / MINOR); if SOUND, list the
failed attacks. Self-refute once before reporting.
```

### What the numbers decide (Job 2's pre-agreed fork — do not improvise later)

The instrument's real output needs ONE live perf-gated capture segment (added to the live
agenda below). Then, per domain over warm frames:

- **guarded_us dominates lane wall (materialise share small, like d53's encode):** the
  acquire/process split is DEAD — bury it in "Known nulls" and spend entirely on cheaper
  acquisition: prefetch distance A/B, d23 locality/coalescing probe results, region-cache and
  primitive work. The budget curve says ~30–40 ms cycle ⇒ ~0–0.1% skips, and the prefetch lane
  already measured d53 ~3× above its memory-latency floor — cheaper acquisition alone may get
  there.
- **materialise share is large (≥ ~35–40% of the top lanes' wall):** design the split — hour-
  boundary raw-byte capture + deferred decode, shipped behind the d53-template shadow mode
  (old inline path vs deferred path, fail-closed on mismatch), lossless by construction or it
  does not ship.
- Either way: re-run the acquisition-budget curve (scripts/burst_corr.py against a fresh
  capture) with projected D to re-check reachability of zero-skip, and remember the pace-
  exogeneity caveat — D≈28 ms ⇒ zero skips is an upper bound on benefit until the exogeneity
  experiment runs.

---

## Live-session agenda (status as of 2026-07-10 late session)

1. Three-phase capture segment: DONE (debug capture, ~60% guarded / ~40% materialise).
2. Prefetch A/B: plumbing landed (1b8ed7d66: `-PrefetchDistRelation` launcher param →
   control file → lane; env vars can't reach a running game). Windows PENDING — run
   palindrome [16,8,0,0,8,16] short production windows, compare
   `lane_wall.per_domain_critical_path_us_total["53"]/frames` + FRAME_END times; G1-fold one
   arm pair for byte identity.
3. d23 locality probe: ATTEMPTED 2026-07-10 — gate-bracket caught the game advancing
   mid-probe, result DISCARDED per protocol. Needs a truly PAUSED save. (The discarded
   pass suggested no adjacent-object merging at gap≤0x100 and 7.7–9x copy amplification at
   useful caps — if that holds on a valid pass, the coalesced reader is likely a NULL per
   the read-count≠time law, but do not decide on a discarded run.)
4. hour-latch paired A/B: first ABBA attempt 2026-07-10 invalidated (game was PAUSED for
   w1/w2; w3 mixed; a staging bug also collided a rerun w1 with w4 — two concurrent
   recorders, both windows discarded). Clean ABBA rerun in flight (`runs/ab_latch3_20260710`).
   6-core portability run still owed.
5. d53 delta_partial paired A/B: **CLOSED — SUPERSEDED** (see Job 0b row).
6. Pace-exogeneity experiment: still owed (fixed save+speed, domain subsets ~40/~80/~151 ms).

Gate-bracket every perf run (the game pause semantics: paused = 1 cold baseline frame; a
mid-window unpause/pause makes the window MIXED — discard, both regimes contaminate).

## QUEUED 2026-07-11 midday — Codex OUT OF CREDITS (user must add; never retry into it)

Diagnosis note: before the clean "out of credits" error surfaced, launches died as
"models manager timeout" and left a ~80-process zombie cascade (+1 wedged 17h daemon,
PID killed). Fleet cleaned. On credits returning, dispatch:

1. **o3-final-net-gate-v2** — resume tid `019f4ebb-0fb6-7cc1-88e8-3755a1ae6b2f` (thread has
   the full prompt as its first message; if resume misbehaves, the prompt is reproducible:
   net-state adversarial gate on commits 8e10e70ff + 28f2043e1 + d5bb02247 — fix-round-2
   closure verification, d69 stale-slot adjudication, revert completeness, sweep-loop
   termination, d43 hold-streak steady state, `_sample` unit-class safety, d53/d60
   non-regression, and whether keeping slot-3 PARTIAL fail-closed is correct).
2. **O5 modset-binding gate** (commit e9f21a1c8, never dispatched): recorder_host::modset —
   attack fail-closed resolution (ambiguous/missing user dir, unreadable descriptors, exe
   hash mid-update race, census key format vs dlc_load.json), manifest field semantics
   (resolved:false = UNKNOWN not vanilla), CLI arg wiring, FFI safety of
   QueryFullProcessImageNameW usage. Live evidence: first run resolved=true mods=0
   dlc_enabled=20/24 matching dlc_load.json exactly.

Thread log: ~/.claude/codex_threads.log.

### AMENDMENT (same day, after live restoration): widen gate 1's commit set

Gate 1 must now cover 8e10e70ff + 28f2043e1 + d5bb02247 **+ ebdc5bc4b + 1fbd9f3a4**.
The last two are LIVE-VALIDATED (d47: 831,039 rows / 0 parse errors / dead_roots 1 on a
509-hour window; d69: status_or 0, 6 groups/frame) but NOT yet adversarially reviewed.
Lossless-critical attack surface for the new commits:
- d47 char-dedup: is GLOBAL char_id the correct dedup key (matches the payload gate's
  seen_characters.insert(char_id))? Can two LIVE distinct characters share a char_id
  (would silently drop one)? Degrade-to-None rule on name/token disagreement; duplicate
  objects disagreeing on COUNTRY (kept root arbitrary — flag if that can misattribute rows).
- d69 vacant/tombstone: stable 0 / high-bit member words skipped as structural non-members
  (tag ceiling 2048 makes them impossible members) — can a live member word transiently
  present stably-high-bit across a double read? Positive out-of-range still fail-closed.

## GATE STORM STATE 2026-07-11 EVENING (plan v2 §1; tree SHA for all four = 319dc641b)

Codex usage limit hit at 15:01 ("try again at 7:06 PM") after two plan-review jobs. NEVER
retry into it. All four prompts are FULLY SELF-CONTAINED (evidence inlined — the read-only
sandbox shell is broken host-wide: CreateProcessWithLogonW 2; Codex must need zero tool
calls). Durable prompt files: `C:\Users\Example\.claude\hoi4_gate_prompts\`.

| Gate | How to dispatch | Status |
|---|---|---|
| G1 final net gate (6 commits incl. d47 dedup + d69 tombstone + O7) | `resume 019f4ebb-0fb6-7cc1-88e8-3755a1ae6b2f` + pipe g1_prompt.txt | QUEUED (credits) |
| G2 modset (e9f21a1c8) | fresh thread + g2_prompt.txt | QUEUED (credits) |
| G4 O8 design review | `resume 019f4f8d-2021-7ca3-9b8d-1ac319ff970a` + re-send hoi4_gate_evidence_g4_prompt.txt (first turn died on the limit BEFORE consuming the prompt) | QUEUED (credits) |
| G5 audit reconciliation (95 branch commits vs this queue) | fresh thread + g5_prompt.txt | QUEUED (credits) |
| G3 serial-wall (1cac48222) | NOT YET WRITTEN — waits on P1b equivalence evidence per plan v2; P1a decision = SHIP NEW (6-window table in plan §2) | HELD |

PLAN SELF-REVIEW (done): tid 019f4f5c-c127-7a90-a995-1c4c97dcc571 — AMENDMENTS REQUIRED,
12 findings, all folded into PRODUCTION_ENDGAME_PLAN.md v2 (59ae8ceb3 + 319dc641b).

## D69 ACTIVE-COUNT RE — BLOCKED ON LIVE ORACLE (opened 2026-07-11, re-gate 019f5066)

d69 slot-level stale classification was retired as unsound (e908a0f61): without the
tech-sharing manager's ACTIVE-GROUP-COUNT field, a non-null slot with a stable non-group
vtable cannot be distinguished (mid-construction live group vs freed-and-reused slot), so
it fails closed → PARTIAL. To restore POPULATED-and-sound d69:
- RE the CTechSharingManager active-group count/size (the fixed 9-slot array model is
  single-fixture and now doubly-refuted). Candidate: a count/size field near
  DOMAIN69_CGAMESTATE_TECH_SHARING_MANAGER_OFF (0x3A0) or the slots are actually a
  pdx::vector with begin/end/count. Needs a live paused deep-war save + the D04 melt oracle
  (9/9 groups) to validate the count offset, then iterate only [0..count) slots.
- Evidence already collecting: d69_stale_vtable_rva_sample names the reusing engine type
  per stale slot (match offline vs rtti_vtable_map.json) — if the "stale" slots all point
  at ONE known type, that itself hints at the manager's internal free-list structure.
- Alternative (if count RE fails): a d69 PARTIAL-upsert fold contract (merge observed groups,
  never full-replace) — lossless-critical fold change, own design + gate, also needs live.
BLOCKED: user is playing (game off-limits); no captures until released.

## D69 ACTIVE-COUNT RE — EVIDENCE LANDED (2026-07-12 tip window tipval_085800)
d69_stale_vtable_rva_sample = 43364200 = 0x295AF68 == CPROVINCE_VTABLE_RVA.derived_from_rva
(UNIQUE match in rtti_vtable_map.json). So the d69 "stale" tech-sharing-group slots hold
RECYCLED CProvince objects from a shared allocator pool — NOT torn tech-group reads. This
CONFIRMS the freelist/pool-reuse hypothesis and kills the fixed-slot-count model: past the
active count, slots hold whatever the allocator reused (here CProvince). d69_fail_vtable_unproven
= 13212 (fail-closed, correct). groups 6606 / members 37434 flowing; flags=512 PARTIAL (accounted).
RE TARGET now concrete: find CTechSharingManager's active-group COUNT field (candidate near
DOMAIN69_..._OFF 0x3A0, or slots = pdx::vector begin/end/count) and iterate ONLY [0..count),
so a vacated slot reused by CProvince is never walked. Validate against the D04 melt oracle
(9/9 groups) on a paused deep-war save. This is now UNBLOCKED for static RE (evidence in hand).

## D47 ZERO-ROWS ON FRESH-LOADED SAVE — dedup fix EXONERATED, prewarm suspect (2026-07-12)
tipval_085800 (tip bb75395c, 2201 game-hours): d47 emitted 0 char rows, flags=768 (SOURCE|PARTIAL),
domain47_adapter_country_roots_seen=0, prewarm_skipped_root_rows=2137, sweep_skipped_ranges=3146.
Every PRIOR window (all on the user's LONG-WARMED game) emitted 400k-1.7M rows / 500-1572 roots.
VERIFIED NOT THE DEDUP FIX (615d24125): country_roots_seen = input.countries.len()
(lanes/characters_leaders_cabinet_live/src/adapter.rs:258) is measured UPSTREAM of the char_id
dedup — 0 countries in means the dedup never ran. Root cause = d47 country-root ACQUISITION
supplying 0 countries to the adapter, consistent with a COLD d47 prewarm on a freshly-loaded
save (this window ran on an orchestrator -start_save=autosave relaunch; all d47-working windows
were on the user's continuously-running game). DIAGNOSIS NEEDED: does the d47 prewarm/sweep warm
within a capture window on a freshly-loaded save, or does fresh-load leave input.countries empty
indefinitely? If the latter, production captures that START on a loaded save have a total d47
coverage hole = real bug. NOT a revert. Confirm on the next fresh-load window (is d47 still 0?).

## D47 ZERO-ROWS — ATTRIBUTION CORRECTED: CONFOUNDED A/B + a VERIFIED latent bug (2026-07-12)
CORRECTION to the entry above: my old-vs-tip A/B was CONFOUNDED. Both windows loaded
"autosave", but the tip window ADVANCED the game ~5 game-months and re-autosaved before the
old-DLL window loaded it — so the two runs used DIFFERENT save states. "old works (1.08M rows,
0 skips) / tip zero (0 rows, 2137 prewarm + 3146 sweep skips)" therefore mixes BUILD and
SAVE-STATE and is NOT a clean build verdict. Also: domain47_sweep_skipped_ranges 0->3146 is
NOT explained by either collector commit's diff (the sweep at lib.rs ~53357-53440 is untouched
by 615d24125/e908a0f61), which points to fresh-load heap instability as a real co-factor.
WHAT IS VERIFIED (code, not run): 615d24125 added country-conflict + unit-leader-conflict as
skipped_root_rows triggers (lib.rs 18311/18341/18367). The prewarm gate at 18549
`if skipped_root_rows != 0 || stable_root_count()==0` was designed for TRANSIENT torn reads:
on a fresh load (had_stable_roots=false) ANY skipped_root_rows>0 sets cache.roots = EMPTY and
schedules a negative-prewarm backoff. A country-conflict is PERSISTENT (same save, same
ambiguous char every scan), so a single stable ambiguous character makes d47 NEVER warm on a
fresh load -> total character+country coverage zero (country_roots_seen=0 because the collector
emits an empty frame, which is why my earlier "dedup exonerated, upstream" call was ALSO wrong).
This is a real latent design bug: a per-root ambiguity should DROP that root (like
excluded_scan_hits) and still emit the other ~4000 characters, NOT gate the whole frame.
OPEN: (a) clean A/B on ONE FROZEN save file (copy autosave to a fixed name; load the SAME file
with tip then old) to separate build from save-state; (b) explain sweep_skipped_ranges 0->3146;
(c) fix design (drop-conflicting-root-only) + gate. DO NOT hand-fix until the mechanism is
adversarially confirmed and the clean A/B run — user directive: verify real before fixing.

## D47 ROOT-CAUSE CONFIRMED (Codex Mode-A tid 019f5387, 2026-07-12) — TWO INDEPENDENT BLOCKERS
Codex confirmed the dedup mechanism AND separated it from the sweep:
BLOCKER 1 (dedup defect, 615d24125): country-conflict char_id (18341) + unit-leader conflict
(18367) increment skipped_root_rows, which the WHOLE-PREWARM gate at 18549 treats identically
to transient torn double-reads. On fresh load (had_stable_roots=false) any skipped_root_rows>0
sets cache.roots=default() (empty), clears sweep_gen, schedules negative-cache rescan; a
PERSISTENT conflict repeats every scan so had_stable_roots never turns true -> d47 stays 0
(country_roots_seen=0, rowless PARTIAL). DESIGN DEFECT: persistent localized ambiguity is
wrongly coupled to the transient whole-frame gate.
BLOCKER 2 (sweep, INDEPENDENT): sweep_skipped_ranges is incremented ONLY in the heap sweep
(VirtualQuery fail 53368, read_bytes_scan page-unreadable 53394, no-aligned-progress 53425) —
615d24125 did NOT touch these. 3146 skips = a SEPARATE fresh-load/save-state heap-coverage
failure. So the confounded live d47=0 CANNOT be attributed exclusively to 615d24125; either
blocker alone can zero d47. (Frozen-save A/B in flight to see which fires on the 1940.12 save.)
FIX DESIGN (verified, not yet implemented): reserve skipped_root_rows for torn/unreliable reads
ONLY; route country-identity conflicts to a NEW ambiguous_root_rows counter (or excluded_scan_hits);
QUARANTINE all roots for a conflicting char_id (no arbitrary winner) and drop only the conflicting
unit_leader mapping — keep the other ~4000 characters; adopt `next` when it has stable roots +
only localized ambiguities. ACCEPTANCE TESTS (Codex-specified, 4): (1) ~4000 valid + 1 conflicting
char_id -> adopts roots, stable_root_count>0, skipped_root_rows==0, ambiguity counter>0, NOT
rowless, ambiguous identity emits under NEITHER country; (2) 2 chars sharing a leader ptr across
countries -> both char roots kept, no leader row for the ambiguous assoc, domain nonempty;
(3) genuinely torn double-read -> skipped_root_rows>0, empty cache, SOURCE|PARTIAL rowless;
(4) zero validated roots -> 18549 stable_root_count==0 branch still fails closed.
BLOCKER 2 still needs its own investigation (why a freshly-loaded save has 3146 unreadable sweep
ranges when a warm game has 0 — real fresh-load d47 sweep fragility affecting production captures
that start on a loaded save).

## D47 — RESOLVED: BUILD EXONERATED (clean frozen-save A/B, 2026-07-12)
Clean same-build test on a FROZEN, stable save (d47ab_frozen.hoi4 @ 1940.12): TIP build
emitted 1,344,265 d47 rows, country_roots_seen=1052, char_rows=7.96M, prewarm_skipped_root_rows=0,
sweep_skipped_ranges=0, sweep_degraded=0, stable_root_count=16.3M. d47 WORKS PERFECTLY on tip.
=> The earlier tipval_085800 d47=0 was NOT a build regression. It was BLOCKER 2 (fresh-load
sweep-coverage): that specific just-loaded early save had transient unreadable heap ranges
(sweep_skipped_ranges=3146) -> coverage_complete()==false (lib.rs 52981: !degraded && skipped==0)
-> the prewarm at 18265-18271 CORRECTLY HELD (fail-closed rootless, refusing to adopt a lossy
character world) rather than emit a silently-reduced snapshot. Arguably CORRECT behavior. The
confounded first A/B (old on a later/warmer autosave) made it LOOK like a build regression; the
frozen A/B overturns that. Blocker 1 (dedup defect) DID NOT fire here (skipped_root_rows=0 — this
save has no persistent country-conflict).
NET: no build regression; no revert. Two follow-ups, both NON-URGENT:
- LATENT BUG (blocker 1, verified by Codex tid 019f5387, fix designed + 4 tests specified in the
  entry above): a persistent country-conflicting char_id would zero d47 on a save that HAS one,
  because 615d24125 routes that ambiguity into the transient whole-prewarm gate (18549). Worth
  fixing opportunistically (decouple to ambiguous_root_rows + quarantine the identity), NOT a
  regression, NOT this session. Worktree lane/d47-ambiguity-decouple created + code understood;
  implementation deferred.
- CAPTURE-OPS (blocker 2): captures that START on a freshly-loaded save may see a d47 warmup gap
  until the heap stabilizes (sweep coverage completes). tipval_085800 never stabilized over 600s
  which is unusual; the frozen save was clean from frame 1. Save/timing-specific, not reliably
  reproducible. Recommendation: warm the game briefly before a production capture, or accept the
  fail-closed d47 hold (zero-silent-GAP: it's accounted PARTIAL, not silent loss).

## D69 ACTIVE-COUNT RE — HYPOTHESIS LANDED (Codex Mode-A tid 019f53b9, 2026-07-12)
The CProvince-reuse evidence (0x295af68) proves INLINE pointer slots (not a vector: a vector's
manager+0x00/+0x08 would be backing-store/end pointers, not recycled objects). Ranked candidates
for the active-group COUNT (relative to the manager at CGameState+0x3A0, slots inline at +0x00):
1. **manager+0x48 = explicit active_count** (0x00 + 9*8 = 0x48, right after the inline [9] array).
   Layout: group_ptr[9] @0x00..0x47, then u32 active_count @0x48 (+ maybe u32 capacity @0x4C),
   OR u64 count @0x48. STRONGLY FAVORED. Live check: stable-read u32 AND u64 at +0x48; require
   == active/valid-slot count; capacity (if @0x4C) in [9,256]; refute if neither width matches or
   it moves independently of live-group count.
2. manager+0x48 = one-past-active END pointer (require end==manager+N*8). Weaker (awkward overlap).
3. manager+0x08 = vector end {begin@0x00,end@0x08,cap@0x10}. REFUTED if manager+0x00 is itself a
   direct group object (the evidence says it is) — but the belt-and-suspenders check is listed.
4. array-list {data@0x00,size@0x08,cap@0x0C}. Weakened by the recycled-object evidence.
INVARIANT (all candidates): stable-read metadata; reject misaligned begin/end + count>256; then
STILL vtable-validate every [0..count) slot; torn/invalid => accounted PARTIAL (never silent).
VALIDATION NEEDS: a paused deep-war save + the D04 melt oracle (9/9 groups) to correlate the
field with the count, PLUS a SECOND state (fewer groups) or a create/remove transition to prove
it's a DYNAMIC count not a constant capacity (the count must change while capacity stays stable).
NEXT CONCRETE STEP (cheap, no oracle needed for a first pass): add a collector telemetry probe
that stable-reads u32+u64 at manager+0x48 as a _sample, run one live window, and compare the
value against the reader's own valid-slot count per frame — if they track, candidate 1 is
live-supported; then confirm dynamic-vs-capacity with the oracle. This turns the fixed-9 walk
into a [0..count) walk = populated-AND-sound d69.

## 2026-07-12 — d69 active-count probe SHIPPED (telemetry-only) + P4 driver fix reviewed
- Probe (#24): manager(CGameState+0x3A0)+0x48 (`DOMAIN69_TECH_SHARING_MANAGER_ACTIVE_COUNT_PROBE_OFF`,
  first word after the 9-slot array) double-read-stable as u32 AND u64 every frame; per-frame
  3-way verdict counters (match / indeterminate when a below-count slot failed / structural
  mismatch) + `d69_probe_fail_above_count`; atomic same-frame packed sample
  `d69_probe_topology_sample` (probe | nonnull<<32 | valid<<41, 9-bit masks). Emission and
  fail-closed semantics UNTOUCHED. Promotion bar (6 legs, incl. fail_above stale-signature
  audit + D04 melt oracle) documented in ~/.claude/hoi4_gate_prompts/d69_probe_harvest.py.
- Adversarial review: Codex Mode A tid 019f53e9-ff39-7c33-add5-aa245b89d4bd. Finding 1
  (empty --scheduled-domain bypasses exact-mode) REFUTED against validate_domain
  (main.rs:2833 rejects empty). Findings 2 (verdict false-match/false-mismatch) and 3
  (`_sample` units pair across different frames) CONFIRMED and fixed in the same commit
  (3-way verdict + packed topology sample). One fix round used.
- Gates: collector 353/0, recorder_host 190+194/0, compact_contract 18/0.

## 2026-07-12 — d47 BLOCKER-1 (latent dedup defect) FIXED + double-gated
Implemented the tid-019f5387-verified design: `Domain47CharacterDedup` (pure bookkeeper) —
country-conflicting char_id => QUARANTINE (kept root filtered post-loop via
`domain47_filter_quarantined_roots`, its leader mapping removed AND tombstoned, all further
hits dropped), cross-country unit-leader claim => mapping dropped + ptr poisoned, both roots
kept. Ambiguities counted in NEW `ambiguous_root_rows` (summary + cache + unit
`domain47_prewarm_ambiguous_root_rows`), DECOUPLED from `skipped_root_rows` — the adoption
gate (extracted `domain47_prewarm_adoption_blocked`) takes torn reads + zero-root only, so a
persistent localized ambiguity can no longer zero the whole domain forever. Zero-ambiguity
saves are behavior-identical (verified: same merge path, same map type/order).
Review round (Codex tid 019f53fb): BLOCKER — quarantine removed the leader mapping WITHOUT
tombstoning, so a later claim re-inserted via the Vacant branch (order-dependent arbitrary
winner). CONFIRMED against the code, fixed (tombstone on quarantine). MINOR — acceptance
test 1 was bookkeeping-only; CONFIRMED, upgraded to integration-style (real admit loop +
production filter + the exact blocker sequence). Re-gate on the same thread: SHIP.
All 4 tid-019f5387 acceptance tests present; collector 356/0.

## 2026-07-12 — d69 COUNT FIELD FOUND (CGameState+0x3AC = active group count)
Live probe series (telemetry-only, reviewed tid 019f53e9) cracked the d69 topology:
- +0x48-count REFUTED (stable non-count constant 0x90038D00B9490FCD).
- CGameState+0x3A0 is NOT a 9-slot inline manager: it is a growable vector of
  CTechnologySharingGroup* — `{ T* begin @+0x3A0; u32 capacity @+0x3A8; u32 size @+0x3AC }`
  (size = high dword of the +0x3A8 u64). The current reader dereferences +0x3A0 to `begin`
  and walks a FIXED 9 slots; slots [size..9) are OUT OF BOUNDS and hold stale/string data —
  the SOLE source of the chronic d69 PARTIAL (every frame badged PARTIAL, domain out of every
  replay).
- EVIDENCE (size == decoded valid-group count, capacity >= size, BOTH saves):
  frozen 1940.12: cap=4 size=4, 4 valid groups (slots 0-3).
  deep-war 1941.02 (coherent tuple read): cap=6 size=5, 5 valid groups (slots 0-4); slots 7,8
  non-null but OOB garbage (the PARTIAL trigger). [frozen coherent re-confirm in flight]
POPULATED-AND-SOUND FIX DESIGN (NOT yet implemented — gated on the melt oracle):
  read count = u32 @ CGameState+0x3AC (double-read-stable); FAIL CLOSED if unreadable, or if
  count > capacity, or capacity implausible (> small cap, e.g. 512). Walk begin[0..count) with
  the EXISTING per-group vtable/id/member validation. A slot < count that fails validation is a
  REAL torn read -> keep PARTIAL (do not silently drop a live group). Slots >= count are simply
  not read (no more OOB, no more chronic PARTIAL). Contract constant:
  DOMAIN69_CGAMESTATE_TECH_SHARING_GROUP_COUNT_OFF = 0x3AC (+ CAPACITY_OFF 0x3A8, BEGIN_OFF 0x3A0).
PROMOTION BAR before switching emission: coherent frozen re-confirm (cap=4/size=4) + a D04 melt
oracle cross-check that size == the TRUE group count on a matching save (guards the case where a
real group fails vtable and size > decoded-valid-count). Until then d69 stays fail-closed PARTIAL
(no data loss — the domain is CORRECT, just unpopulated).

## 2026-07-12 — d69 count-field: coherent frozen re-confirm BLOCKED by advancing save (not a refutation)
The hardened coherent probe (read_u64_stable per word, DLL f4360971) still emitted NO
d69_vec_* triple on the "frozen" fixture across two windows. Cause identified, NOT a defect:
the fixture ADVANCES once unpaused (1940.12 -> 1941.02) and GREW from 4 to 5 tech-sharing
groups during the window (valid_slot_mask 0x0f -> 0x1f), so `size` at +0x3AC mutates mid-read
and the double-tuple coherence guard correctly rejects every frame. A clean coherent frozen
point needs a PAUSED game (static size) — user-gated (no reliable keystroke to the fullscreen
game). 
EVIDENCE STANDING for CGameState+0x3AC = active count (unchanged, still strong):
- deep-war 1941.02 COHERENT same-frame (d69r4tuple): begin=heap ptr, cap=6, size=5, 5 valid
  groups. size==groups, cap>=size.
- frozen 1940.12 INDEPENDENT sample (d69r3vec): +0x3A8 u64 = 0x400000004 => cap=4 size=4,
  4 valid groups. size==groups, cap>=size.
- semantic invariant holds in every window: cap>=size, size==decoded-valid-group-count.
PROMOTION GATE remains the D04 MELT ORACLE (validates size == TRUE group count, covering the
case a real group fails vtable so size>decoded-valid) + one coherent read on a PAUSED game.
The +0x48 verdict counters (d69_probe_count_match/mismatch) now test the REFUTED +0x48
hypothesis and are MEANINGLESS noise — the harvest "STRUCTURALLY VIOLATED" line reads those,
ignore it; the +0x3AC finding is judged by the VEC line + the table above. d69 stays
fail-closed PARTIAL = CORRECT (zero data loss), just unpopulated, until the gate clears.

## 2026-07-12 — d69 +0x3AC count field: FINAL evidence state (RE thread closed for the session)
Added an independent stable read of the +0x3A8 size word (d69_vec_size_word_sample) as a
belt-and-suspenders alongside the coherent triple. On the CURRENT frozen process it ALSO
returned None: gs+0x3A8 is unreadable while gs+0x3A0 (8 bytes earlier) reads fine — a
per-process PAGE-BOUNDARY artifact (gs+0x3A0 landing at a page edge), NOT a refutation (an
earlier frozen process read gs+0x3A8 = 0x400000004 cleanly). CORROBORATION from the paused
frame regardless: the reader decodes exactly 6 valid groups (slots 0-5) then fail2 at slot 6
(vtable_word 0x0, stably unreadable past-the-end) — the array empirically ENDS at the group
count, independent of reading the size field.
STANDING EVIDENCE for CGameState+0x3AC = active group count (3 saves, all consistent):
- deep-war 1941.02 COHERENT (d69r4tuple): cap=6 size=5, 5 valid groups.
- frozen 1940.12 INDEPENDENT (d69r3vec): +0x3A8 = 0x400000004 => cap=4 size=4, 4 valid groups.
- frozen 1941.04 PAUSED (d69paused/2): 6 valid groups, array ends at slot 6 (OOB at 6).
size == decoded-valid-group-count in all three; cap >= size where readable; the reader's
fixed-9 walk reads [count..9) OOB = the sole chronic-PARTIAL source.
PROMOTION GATE (unchanged, both non-live): D04 MELT ORACLE (size == TRUE group count, covering
a real group that fails vtable) + a coherent read on a process where gs+0x3A8 isn't page-edged.
d69 stays fail-closed PARTIAL = CORRECT (zero data loss), just unpopulated, until then. RE
thread CLOSED for this session — the fix is designed, the offset is identified, the gate is a
tool step, not a live one.

## 2026-07-12 — d69 MELT ORACLE PASSED (D04_1944, 9/9 groups + 34/34 members) — count field CONFIRMED
User loaded D04_1944 (paused). Live d69 (d69_d04) vs parse_world.py melt oracle:
- LIVE: 9 valid groups (valid_slot_mask 0x1ff, fails=NONE, status_flags 0x0 CLEAN), 34 total
  members. vec header read cleanly this process: d69_vec_size_word = 0x900000009 =>
  capacity=9, SIZE(+0x3AC)=9.
- ORACLE (rakaly melt -> parse_world tech_sharing_group): 9 groups, member counts
  [2,2,10,1,7,2,2,2,6] = 34 total. ids: BUL_axis_research, BUL_italian_research,
  CHI_technological_cooperation, FIN_joint_r&d, HUN_joint_air, MEX_USA, SOV_soviet_academies,
  VIC_learn_from_the_enemy, commonwealth_research.
- MATCH: groups 9==9, members 34==34, size(+0x3AC)==decoded==oracle==9 ON THE SAME FRAME.
Combined with the count-field tracking on smaller saves (frozen 4, deep 5, frozen 6), the
+0x3AC finding is now ORACLE-CONFIRMED at the top (fully-populated 9/9) AND tracking below.
The zero-data-loss concern (a real group hidden behind a vtable failure so size>decoded) is
retired here: decoded==oracle==9, no group missed. GATE CLEARED — implement the populated-sound
[0..size) walk (fail-closed if size unreadable or size>capacity). On D04 the fix is a NO-OP
(size=capacity=9, walk [0..9) == current fixed-9); its EFFECT (dropping OOB PARTIAL) shows on
<9-group saves. Oracle json: scratchpad/d04oracle/world.json.

## 2026-07-12 — d69 POPULATED-SOUND FIX committed (live-validated on D04; re-gate QUEUED on Codex outage)
Implemented the [0..size) walk bounded by CGameState+0x3AC (active group count). Reviewed by
Codex tid 019f5454 which found 4 real findings (all traced against the code + addressed):
 1. BLOCKER fallback clean-truncation -> force_partial when active_count None (fold retains prior).
 2. BLOCKER incoherent header -> COHERENT tuple read (twice-identical) + post-walk re-read.
 3. MAJOR stale extra group / null-in-active -> slot[size] peek + null-in-[0..size) force_partial.
 4. MAJOR capacity ceiling -> constrain SIZE not capacity (cap=512/size=10 takes sound path).
LIVE VALIDATION (d69fix_d04, paused D04_1944, DLL 426872f7): sound path RAN
(active_walk_count=9), emitted 9 groups / 34 members, status 0x0 CLEAN, fail_reasons none,
vec_size_word 0x900000009 (cap=9/size=9) == melt oracle. Self-traced stale-low: peek catches
it (slot[size] is a live group when true>size -> force_partial -> retain prior). Stale-HIGH
phantom = narrow bounded-negative no worse than old fixed-9 (needs an implausibly stably-wrong
size; reserved non-group slots already fail-close to PARTIAL). collector 356/0.
RE-GATE STATUS: Codex returned empty 4x (clean exit, NO usage-limit marker = transient service
outage, not credits). Prompt saved at scratchpad/d69regate_fresh_full.txt — RESUME the fresh
re-gate when the service recovers; if it finds anything, one follow-up fix round. The change is
strictly MORE fail-closed than the reviewed baseline (only adds PARTIAL triggers), so it cannot
introduce data loss vs. the reviewed version.
OWED before production tag: (a) the re-gate; (b) one live confirm of the IMPROVEMENT on a <9
save (frozen: expect N groups CLEAN instead of chronic PARTIAL) — the sound path is already
proven to run + emit correctly on D04, the <9 case is the same path with a smaller count.

## 2026-07-12 — d69 IMPROVEMENT LIVE-PROVEN on a <9 save (before/after CLOSED)
d69fix_frozen (frozen fixture, 4 groups, DLL 426872f7, 351 frames): per-frame 4 groups /
24 members, active_walk_count=4, vec size=4/cap=4, valid_slot_mask 0xf, status_flags 0x0
CLEAN, fail_reasons NONE. The OLD fixed-9 walk read slots 4-8 OOB and badged EVERY such
frame PARTIAL (d69 absent from every under-populated replay); the fix walks [0..4) and emits
the 4 real groups CLEAN. Together with D04 (9/9 no-op, oracle-matched) this is the full
before/after: fully-populated saves unchanged, under-populated saves go PARTIAL->CLEAN.
d69 domain COMPLETE pending only the Codex re-gate formality (retried after the outage).

## 2026-07-12 — Codex RECOVERED; stale G1/G2/G4/G5 queue assessment
Codex service back (watcher probe 2). The d69 re-gate (CURRENT prompt, built today) auto-fired
on recovery. The credits-outage gate queue (G1 tid 019f4ebb, G2 e9f21a1c8, G4 tid 019f4f8d,
G5) pins tree SHAs from BEFORE today's ~15 commits — firing them as-is reviews STALE diffs.
Moreover today's fresh reviews already adversarially covered the CURRENT versions of the paths
those gates touched: d47 (019f53fb BLOCKER confirmed+fixed, re-gate SHIP), d69 (019f53e9 +
019f5454), P4 (019f53e9). So G1 (old d47 dedup + d69 tombstone) is largely SUPERSEDED by the
blocker-1 rewrite + populated-sound fix. ACTION: do NOT blind-fire the stale prompts; regenerate
against current HEAD only for the parts NOT already covered (G2 modset resolver, G5 audit
reconciliation), and treat G1/G4 as superseded pending a spot re-check. Restored-Codex priority
is the HELD gated code work (R2 bound-trip-samples, C1 stage-A), gated fresh.

## 2026-07-12 — d69 re-gate 019f5467 (auto-fired on Codex recovery): 1 BLOCKER + 2 MAJOR, fix round DONE
The watcher auto-fired the queued d69 re-gate on Codex recovery. It found REAL issues the outage
had left ungated (vindicating the gate):
- BLOCKER: collector emits up to 256 groups but host consumers (world_fold.rs:5078, main.rs:14080)
  hard-rejected >9 -> a clean 10-group frame dropped entirely. FIXED: both caps = shared
  DOMAIN69_TECH_SHARING_MAX_GROUPS (256). Commit 18a9f44ec.
- MAJOR: slot[size] peek false-PARTIAL'd after a shrink (spare slot retains popped ptr, can't be
  distinguished from a live group). REMOVED (coherent header + post-walk re-read still catch torn
  size).
- MAJOR: stale-HIGH phantom documented as pre-existing bounded-negative (no worse than old fixed-9).
All 3 verified against the code before fixing (user directive). collector 356/0, host 190+194/0.
RE-GATE 2 dispatched (tid d69_regate2) to confirm the fix round. NOTE: this proves the value of
NOT trusting the outage-time self-review — the re-gate caught a real >9 regression the live D04/
frozen tests (both <=9 groups) could not surface.

RE-GATE 2 VERDICT (tid 019f547f-feae-7830-b929-9ee48b2424df): **SHIP — no findings.** Verified:
10..=256 accepted end-to-end via the shared ceiling (both host consumers on the same const);
fold attaches every declared group without truncation; slot[size] peek gone (shrink-spare no
longer forces persistent PARTIAL); coherent-header/fallback-PARTIAL/null-active-slot/post-walk
re-read all intact; stale-HIGH no worse under the validated vector contract (documented
bounded-negative); no new panic or data-loss path; 10-group/257 regression test passed. Codex's
sandbox full-run failures (175/190, 160/194) were all PermissionDenied on temp-file-writing
tests, not assertions — my own full runs are green (356/0, 190+194/0, 18/0). Verify-findings
pass also corrected an overclaiming comment in my own fix (push-window stable-lag stale-low is
a bounded residual, dff1b6ab4). d69 fix round CLOSED, double-gated.

## 2026-07-12 afternoon — G2-regen + G5-regen verdicts (both ran on current HEAD 2818b2f35)

### G2-regen (modset, tid 019f5490-f030-7242-8534-a50a1b0fdc36): REFUTED — 5 BLOCKER + 2 MAJOR
Fresh review of modset.rs after both fix rounds (b24ac1ce3, e908a0f61). Regression table: 7 of
14 prior findings CLOSED; G2-1 (content hashing), G2-3/G2-5 (process binding / override
corroboration) NOT closed; G2-4 exe-image residual documented-partial. New findings:
1. BLOCKER unicode-escaped duplicate keys ("enabled_mods") bypass the raw-slice duplicate
   scanner while serde keeps-last -> resolved:true vanilla. Fix: compare DECODED key names.
2. BLOCKER --pdx-user-dir override (and stale probed dir) never corroborated against the
   attached process -> wrong-but-well-formed dir resolves vanilla. (G2-3/G2-5 unclosed.)
3. BLOCKER fingerprint (count,bytes,max-mtime) systematically non-unique (rename/same-length
   edit/same-second edits invisible). Fix: SHA-256 over sorted paths+bytes, bracketed.
4. BLOCKER mod/DLC disk evidence read AFTER process start still resolves (Workshop update
   between launch and attach binds v2 while the process runs v1). Fix: any input at/after
   process start fails closed (interim; real fix = loaded-set evidence).
5. BLOCKER dlc_load.json stability check is mtime-only -> same-mtime atomic replacement
   (OneDrive sync) passes. Fix: file-identity + double-read/hash bracket.
6. MAJOR descriptor with both path= and archive= (or repeated keys) accepted last-wins without
   engine-precedence proof. Fix: reject ambiguous descriptors.
7. MAJOR repack path (main.rs ~1123/1911/2093) bypasses modset resolution AND manifest modset
   validation -> can republish legacy/absent/pre-fix modset shapes. Fix: repack requires a
   current-schema modset object (resolved bool; unresolved => partial_evidence-only shape).
Failed attacks recorded in out_g2_regen.txt (kept). FIX ROUND OWED (queued below vs usage limit).

### G5-regen (audit reconciliation, tid 019f5491-014b-74a3-8740-5809f231d0ec): FAIL + roadmap
65 commits in 319dc641b..HEAD: 20 CODE / 45 DOC-ONLY; only 6 CODE demonstrably covered by
queue-recorded verdicts; 2 under G2-regen; 12 lack a qualifying verdict (2 = exact M1
edit/revert net-zero pair). Regenerated blocking list (by lossless-criticality):
1. container/fold/B3 net gate at current HEAD (old chain + 70eeeefca/9810a7e9b/4b7f15c11/
   4912c05ff + fold-perf lane + a09c772db zstd19);
2. G1-residue gate (d43 part of 615d24125, O7 2a9dc5091, 11b0cc9c3/b0897b457, 55287bd6a);
3. G3 serial-wall (1cac48222 + restore 07ef3cad9) — NOW DISPATCHABLE (P1b resolved by
   construction; queue state was stale);
4. modset (in-flight G2-regen -> now the fix round above);
5. exact-domain scheduling + surviving probe (6d666fa1a, 49c0228e7) — small fresh gate
   (019f53e9's fixes never got a recorded clean re-gate and 49c0228e7 changed code after);
6. prefetch: append recovered 019f4b91 verdict (done below) + default 16->8 into F5;
7. C0 census tooling gate (3ecf9f7eb, synthetic-manifest adversary);
8. M1 pair: administrative net-zero disposition (done below).
Ledger-integrity findings fixed by the administrative records below.

### ADMINISTRATIVE VERDICT RECORDS (G5-regen findings D1-D3: verdicts existed only in
AUDIT_RECONCILIATION.md / session records; tids verified in ~/.claude/codex_threads.log)
- 019f5079-1662-73c1-b385-27eabe1024c2 = the stream-container NET GATE, FOUR ROUNDS on
  trees e908a0f61 -> 70eeeefca -> 9810a7e9b -> 4b7f15c11: 12 findings total (mid-record EOF
  Ok; repack laundering; unbounded payload_len; syntactic closure; unguarded skip-fold seal;
  agreement gaps; single-slot open-hour tracking; under-cap swallow; same-shape source views;
  terminal swallowing; FNV caveat -> per-type payload bounds + SHA-256 agreement), each fixed
  same-day (final code commit 4912c05ff). The 4th round ended with fixes, NOT a clean SHIP —
  G5-regen is right that F5 owns the final integrated verdict on the current tree.
- 019f5078-fa99-7570-8fb9-4b9676f16a3a = M1 buildings Arc/RwLock code-soundness gate,
  verdict SOUND (5 attack classes failed). SUPERSEDED ADMINISTRATIVELY: 97a5d902c was exactly
  reverted by 0a9771955 (git-verified empty pair diff); M1 is NOT in HEAD. Disposition:
  net-zero pair, no runtime gate owed unless M1 is reintroduced; status docs must stop saying
  M1 is shipped.
- 019f4b91-0528-7223-8c7e-2d40f7192fe0 = prefetch-restamp verdict recovery: REFUTED 2
  (substring key-aliasing in the control parser; MAX-aggregated applied-label on mixed
  streams); BOTH already fixed in 8487be6a7 (structural tokenizer; *_applied suppressed when
  *_mixed) — orchestrator re-verified against source 2026-07-11. Remaining prefetch item =
  default 16->8 (3a80881f2) into F5.
- G3 state correction: P1b is RESOLVED BY CONSTRUCTION (paused state immutable /
  order-independent; 55/55 domains emitted — plan §2). G3 serial-wall gate is DISPATCHABLE,
  no longer held.

## QUEUED on Codex usage limit (hit 2026-07-12 14:46, resets ~18:52 — never retry into it)
1. R2 bound-trips gate (commit 903461fde): prompt ~/.claude/hoi4_gate_prompts/
   r2_bound_trips_gate.txt; failed-turn tid 019f54a6-2ac0-7992-910b-16a9e05eb01d (turn.failed
   on usage limit — re-dispatch fresh from the prompt file, do not resume).
2. Modset fix round re-gate (after the G2-regen findings above are implemented).
3. G3 serial-wall gate (1cac48222 + 07ef3cad9, P1b by-construction argument + 55/55 artifact).
4. Exact-domain + surviving-probe gate (6d666fa1a + 49c0228e7).
5. G1-residue gate (d43/O7/backstop surviving code).
6. C0 census tooling gate (3ecf9f7eb).
(F5 container/fold/B3 net gate stays the integrated final word per plan §5.)

## 2026-07-12 afternoon — session progress: R2 shipped, modset fix round shipped, C1 stage A mechanism shipped
- R2 bound-trip attribution SHIPPED (903461fde): compact_contract::bound_trips registry +
  bt_* probe-domain units + compact_stream.bound_trips manifest section; ~28 CLAMP-DROP
  sites wired (skill/focus/tech/2048-tag/1024-tag/faction). Gate queued (usage limit).
- Modset fix round SHIPPED (d46ead1de): all 7 G2-regen findings closed (decoded-key dup
  scan, live-log user-dir corroboration, SHA-256 content binding w/ double-pass bracket,
  evidence-predates-launch checks, dlc_load double-read, descriptor location conflicts,
  repack modset schema validation). Re-gate queued.
- C1 stage A chunk 1/2 SHIPPED (727f82c69): status_origins registry (27 site ids from the
  C0 dossiers), collector per-frame drain to so_*/so_*_sample units, recorder
  compact_stream.status_origins {value, first_hour, last_hour} ledger.

### C1 stage A chunk 2/2 — SITE WIRING (next work item; ids + anchors)
Wire record_status_origin(_sample) at these collector/src/lib.rs anchors (line refs at
tree 727f82c69; ids in compact_contract/src/status_origins.rs):
- FUNNEL-BODY recordings (one edit covers all callers; sample = incoming status_flags or
  packed reason): domain53_source_unavailable_read (fn 19887) -> SO_D53_SOURCE_UNAVAILABLE;
  capwar60_source_unavailable_read (fn 21833) -> SO_D60_SOURCE_UNAVAILABLE (sample should
  pack/ hash failure_reason — the dossier flags reason-erasure);
  naval_combat_live_source_unavailable_read (23393) -> SO_NAVAL_SOURCE_EMPTY;
  compact_naval_combat_live_worker_panic_read (23409) -> SO_NAVAL_WORKER_PANIC.
- PER-CALL-SITE recordings where one funnel serves several causes:
  naval_combat_live_partial_read callers: 23431 (incoming failure-class = INHERITED),
  torn-walk callers (dossier 23029/23066 shifted; anchor NavalWalkError::Torn) = TORN_WALK,
  member/OOB join (anchor Torn|Degraded join) = JOIN.
- d60: inherited PARTIAL (dossier 21409 anchor), torn-row 21571 anchor ("never emit a frame
  that silently dropped"), worker panic 21611 -> SO_D60_PARTIAL_INHERITED/_TORN_ROW/
  SO_D60_WORKER_PANIC.
- d53: partial inherited (19924 anchor), partial_frame skipped_rows!=0 vs
  relation_shadow_mismatches!=0 (split!), worker panic -> the four SO_D53_* ids.
- d54 five routes 27110/27154/27166/27174/27250 anchors in read_compact_army_battleplans_
  depth_live -> the five SO_D54_* ids.
- d48 23422/23437/23463 anchors -> SO_D48_* ids (sample at 23463 = packed
  {hard_failure_flags, gate_ok} per the dossier's d48_status_origin spec).
- d45 anchor decisions.targeted_timed_live.structural_failure -> SO_D45_...
- d65 37474 rows.is_empty() -> SO_D65_SOURCE_EMPTY_NO_WITNESS; 37480 dropped_ambiguous ->
  SO_D65_PARTIAL_DROPPED_AMBIGUOUS (sample = dropped count).
- d55 TBR site 25310 anchor (last_frame_rebaselined) -> SO_D55_TRANSITION_BASELINE_RESET.
- frame CONTROL_INVALID 14247 anchor (build_compact_extraction_gate_records) ->
  SO_FRAME_CONTROL_INVALID (sample = scheduled_domains_valid packed word if cheap).
Then: cargo test collector+recorder; Codex gate the combined stage-A diff when credits
return; a Debug live window afterwards closes C0 round-3 items 1-5 from ONE manifest.

## 2026-07-12 — C1 stage A COMPLETE (both chunks shipped, gate owed)
Chunk 1 (727f82c69) mechanism + chunk 2 (c348504c3) all 27 sites wired (28 call sites; see
the commit messages for the per-domain map). QUEUED GATE ADDENDUM: the stage-A gate (queued
item 7) covers BOTH commits as one diff; attack surface = acceptance-behavior identity at
every wired site (instrumentation must not change any accept/reject outcome), sample packing
correctness (d54 21-bit fields, d48 gate_ok bit 32, naval reason hashes), drain/emission
identity with the bt_* pattern, and recorder hour-span aggregation. After the gate: ONE live
Debug window closes C0 round-3 items 1-5 from compact_stream.status_origins alone.

## 2026-07-12 evening — Codex RECOVERED early; ALL SIX queued gates dispatched in parallel
User confirmed recovery; watcher killed (would have double-fired R2). Dispatched with 5-10s
stagger, tids in ~/.claude/codex_threads.log as they complete, prompts + outputs durable in
~/.claude/hoi4_gate_prompts/:
1. r2-bound-trips-gate-v2 (commit 903461fde) -> out_r2_gate.txt
2. modset-fixround-regate (d46ead1de vs the 7 G2-regen findings) -> out_modset_regate.txt
3. c1-stage-a-gate (727f82c69 + c348504c3) -> out_c1_stagea_gate.txt
4. g3-serial-wall-gate (1cac48222 + restore 07ef3cad9; P1a SHIP-NEW + P1b by-construction
   inlined as evidence; running-game temporal-skew adjudication requested) ->
   out_g3_serial_wall.txt
5. exact-domain-probe-gate (6d666fa1a + 49c0228e7 SURVIVING state; incl. dead-probe-unit
   and 9-bit-mask-vs->9-groups checks) -> out_exact_domain_probe.txt
6. g1-residue-gate (d43 part of 615d24125, O7 2a9dc5091, O2 11b0cc9c3+b0897b457,
   55287bd6a; known-empty full-replace is the named top risk) -> out_g1_residue.txt
Verdicts to be recorded here as they land; fix rounds follow the ONE-round cap.

## 2026-07-12 evening — ALL SIX GATE VERDICTS IN (1 SHIP, 5 REFUTED, ~24 findings)
Full verdicts in ~/.claude/hoi4_gate_prompts/out_*.txt; tids in the threads log.

### G3 serial-wall (tid 019f54c9-b87b): **SHIP** — CLOSED
Byte/fold identity verified (fixed emit order; only PERF-record values vary); running-game
temporal skew adjudicated lawful under the documented bounded-skew contract; alias safety
incl. the NEW registries (readers joined before drain); panic semantics preserved;
07ef3cad9 restoration git-proven blob-identical. 1cac48222 leaves the blocking list.

### R2 bound-trips (tid 019f54c7-3a5e): REFUTED — 3 BLOCKER + 1 MAJOR
1. count-published-before-raw race loses attribution (FIXED 639661ec1 + stress test; also
   applied to status_origins which shared the defect).
2. Unwired live gates remain: collector d66 occupied-tag 38641 + garrison source 38914;
   occupation lane collector_module 385 (country count), 666 (tag), 840 (owner/controller);
   plus a mechanical lane-crate sweep owed. -> fix round.
3. PRE-EXISTING: compact_replay_verification_gates counts probe-domain-250 records; the
   recorder manifest excludes them -> the independent battery cannot agree with any stream
   carrying lane-wall probes (VERIFIED: no 250-exclusion in its scan loop). -> fix round.
4. registry test races (FIXED 639661ec1).

### exact-domain/probe (tid 019f54c9-eb31): REFUTED — 1 BLOCKER + 2 MAJOR
1. BLOCKER: unknown --scheduled-domain names accepted syntactically; coverage derives via
   filter_map -> typo = silently narrower expected set; empty expected set skips domain
   checks entirely (gate-passed empty replay constructible). Fix: fail-closed registry
   resolution at parse + coverage; test battery.
2. MAJOR: refuted +0x48 probe still emits authoritative-looking units (delete constant,
   reads, verdict counters, topology sample).
3. MAJOR: 9-bit topology masks incompatible with [0..size) walk (>9 groups misreported);
   d69_vec_end/cap_sample labels wrong vs confirmed +0x3A8/+0x3AC layout. Delete obsolete;
   keep vec_size_word + active_walk_count + current-walk failure samples.

### C1 stage-A (tid 019f54c7-c9cc): REFUTED — 5 MAJOR + 1 MINOR (no semantics BLOCKER;
insertion-only confirmed)
1. Missing producers: d53 post-build gate SOURCE (20883), d48 lane-set PARTIAL-only frames
   + worker panic, d54 missing-dependency/panic helpers, d65 pre-rows early SOURCE returns.
2. Last-value samples lose multi-cause evidence per hour-span; deciding units short of the
   dossier spec (witnesses for d53/d60/d65; control-transition trace fields).
3. DRAIN GAP: origins drain only inside the lane-wall probe emission (lane_spans-gated;
   early returns skip it) -> counts leak to later frames/hours or die at shutdown; NO_CLOCK
   frames stamp hour 0 into spans. Fix: unconditional frame-finalization drain + missing-
   hour sentinel.
4. d45 counts per failing COUNTRY not per frame (dossier specifies _frames).
5. d54 21-bit packing truncates silently at 2^21 (emit 3 u64 units or saturate+overflow bit).
6. Tests: no wire-path encode->decode->manifest test; registry test races (races FIXED
   639661ec1; wire test owed).

### modset re-gate (tid 019f54c7-97a2): REFUTED — 4 BLOCKER + 1 MAJOR
Closure table: findings 1 (unicode dup), 5 (dlc_load double-read), 6 (descriptor conflicts)
CLOSED; 2/3/4/7 not closed:
1. Log-mtime corroboration insufficient: ANY writer touching logs/ post-launch corroborates
   a wrong dir (no the game instance needed). Fix: process-specific evidence (open log handle
   owned by PID / command-line), keep mtime as liveness only.
2. Namespace changes invisible: rename preserves file mtimes (dir mtimes unchecked) ->
   post-launch renamed tree binds; EMPTY DLC census with no timestamp resolves. Fix: include
   directory mtimes in walks + launch checks; identity-stable double census; empty census
   fails closed.
3. Repack/writers REREAD the manifest from disk at pack time (validated Value discarded;
   also the non-repack writers 3764/3942) -> post-validation replacement gets embedded.
   Fix: serialize the validated in-memory Value; never reread.
4. resolved:true bypasses ALL schema validation on repack (test even blesses a bare shape).
   Fix: full resolved-schema validation + require errors empty.
5. MAJOR: content walk follows directory reparse cycles forever (no visited set; only files
   bounded). Fix: reject reparse points / visited-identity set + dir bound.

### G1-residue (tid from threads log): REFUTED — 4 CRITICAL + 1 HIGH + 1 LOW (all NEW,
pre-existing silent-loss class — the worst findings of the storm)
1. d43 fold accepts PARTIAL frames as authoritative full-replacement (first-hold PARTIAL
   folds A/B while omitting unseen C). Fix: fold treats d43 PARTIAL as unknown/retain.
2. d43 CLEAN-empty authorized by an ASYNC snapshot (30s-stale Some([]) full-replaces live
   contracts allocated since). Fix: no CLEAN empty from async heap snapshots — needs a
   current-frame witness; until then fail closed.
3. d17 fold ignores its status word ENTIRELY (real failures fold as authoritative). Fix:
   gate on COMPACT_STATUS_FAILURE_MASK at fold; WAR_SEGMENT_ABSENT alone stays acceptable.
4. Epoch reset not honored by consumers: d47 hold+12-frame backoff stays CLEAN across a
   sweep-thread restart (new-epoch chars silently missing); d60 keeps consuming an old
   validated discovery at generation 0. Fix: epoch-bind consumer caches.
5. HIGH: work-implies-rows is whole-run aggregate (1 early row masks total later loss) +
   counter families incomplete. Fix: per-frame adapter_work_without_emission accounting.
6. LOW: O7 WAR_SEGMENT_ABSENT missing on the country-worker panic fallback.

### FIX-ROUND PLAN (severity order; one Codex round already consumed by these verdicts —
remaining fixes are orchestrator-implemented, re-gates batch into F5 or per-item)
P0 (live silent loss): G1-residue 1-4.
P1 (false-green/verification): exact-domain 1 (unknown domains), modset 3+4 (manifest
    reread + resolved bypass), R2 3 (probe-domain in verification gates), G1-residue 5.
P2 (mod-robustness completeness): R2 2 (unwired sites + lane sweep), modset 1+2+5.
P3 (evidence quality): stage-A 1-5, exact-domain 2+3 (probe deletion), G1-residue 6.
DONE: R2 1+4 / stage-A 6a (639661ec1); G3 CLOSED SHIP.

## 2026-07-12 late — fix-round progress ledger (verdicts above; every finding code-verified first)
DONE (commits):
- 639661ec1 R2-1 ordering + R2-4 test races (both registries) — then SUPERSEDED for
  status_origins by a521cc827's packed-word fix: my first flag-cell repair was WRONG and the
  new stress test caught it live under parallel-suite load (count and sample-flag must travel
  in ONE atomic word; separate cells let a drain consume the flag against the wrong batch).
- a521cc827 P0: d17 fold status gate (+3-frame test); d43 fold PARTIAL -> retain (+rewritten
  test that previously PINNED the refuted fold-with-omissions behavior); d43 empty always
  fail-closed (warm-known-empty kept visible via 2 new status-origin ids 27/28); O7 bit on
  the country-worker panic fallback.
- a7b5bf64f P0: sweep-EPOCH binding (d47 prewarm cache poisons dead-epoch roots immediately —
  the reset-empty snapshot passed coverage_complete() and the steady-state early return
  served them CLEAN forever; d60 tracker binds validated_epoch through the spawn handle and
  drops the discovery on mismatch). d53 verified already fail-closed. TEST DEBT: epoch arms
  lack pure unit tests (need CollectorState harness) — re-gate to adjudicate.
- 636673d96 P1: exact-domain BLOCKER — unknown --scheduled-domain names fail closed at parse
  (registry resolution) + coverage derivation can never silently narrow (+battery).
- 0d874dad6 P1: modset-4 — resolved:true repack requires the complete evidence schema
  (build hash, provenance, timestamps, per-mod descriptor+content sha, dlc arrays; the old
  test blessed the bare shape and was rewritten).
- c8922ba88 P1: modset-3 — pack-time validation of the ACTUAL embedded manifest bytes at all
  three reread sites; 8 test fixtures gained current-schema unresolved modsets.
REMAINING (in order): R2-3 verification-gates probe-domain-250 exclusion (+stream test);
G1-residue-5 per-frame work-implies-rows; P2 = R2-2 unwired lane/d66 bound sites + lane
sweep, modset-1 (process-specific user-dir evidence: open log handle by PID), modset-2
(dir mtimes in walks + empty-DLC-census fail-closed + identity-stable double census),
modset-5 (reparse-cycle guard); P3 = stage-A 1-5 (missing producers, per-cause ledger
shape, unconditional frame-finalization drain + NO_CLOCK hour sentinel, d45 per-frame,
d54 u64 units), exact-domain 2+3 (delete +0x48 probe + obsolete topology/vec units),
G1-residue 6 done / 5 pending. Re-gate the whole round when done (one batch).

## 2026-07-12 night — FIX ROUND COMPLETE (all six gate verdicts processed; batched re-gate dispatched)
Every finding across the 5 REFUTED gates is implemented + code-verified-before-fixing +
suite-green. Commit map (newest first):
- 250323bbd stage-A finding1 (4 producers wired); 4775bed01 stage-A 4+5 (d45 per-frame,
  d54 saturate); ad7f1ee93 stage-A finding3 (unconditional drain + NO_CLOCK sentinel);
  9840a2aaa exact-domain 2+3 (probe deletion); e0bde5136 modset1 (RM ownership);
  690a19bc7 modset2+5 (dir-mtime/empty-census/reparse); c8922ba88 modset3 (pack-time
  validate); 0d874dad6 modset4 (resolved schema); 636673d96 exact-domain1 (unknown
  domains); 04f2a9391 G1res5 (per-frame work-implies-rows); 511bb1882 R2-3 (probe-domain
  exclusion in verify gates); a7b5bf64f G1res4 (epoch binding); a521cc827 G1res1/2/3/6 +
  d43/d17 folds; 639661ec1 R2-1/4 ordering+test races; 9ba5506af + 690a19bc7 R2-2
  (unwired bound sites).
DEFERRED (explicitly, not point fixes): stage-A finding1 residue (d48 lane-partial +
d65-early producers — need reader inventory) + finding2 (per-(site,cause) ledger SHAPE =
C1 stage B/C schema). TEST DEBT for the re-gate to judge: epoch-mismatch arms (need
CollectorState harness), modset RM symlink refusal (Windows dev-mode), politics_window
parallel flake (pre-existing).

### BATCHED RE-GATE dispatched (whole fix-round diff a679df5b8..HEAD)
Attack the fixes for regressions + the deferred/test-debt items. If SHIP, the audit
blocking list drops to: F5 container/fold/B3 net gate + C0 tooling gate + G4 O8 design
(next stage). Verdict recorded here on return.

### BATCHED RE-GATE VERDICT: REFUTED (tid 019f551c-1fd4-7f61-8732-9911691d19f3)
3 findings survived; 12 attack areas failed (d17 offset/mask, d43 retention + no
CLEAN-empty path, d47/d60 epoch poison incl. same-epoch bump, packed-CAS race-safety +
63-bit headroom ~292yr, both frame-end drains exactly-once + monotonic sequences,
unknown-domain parse fail, probe-250 routing parity, probe deletion clean, d45 frame-local
latch, d54 saturation math, bound-trip outcome preservation, healthy-empty-save no-FP).
Deferred items ADJUDICATED legitimately deferrable (d48/d65 producers + ledger shape =
attribution not loss; epoch test debt = debt not defect; politics_window flake unrelated) —
EXCEPT RM symlink debt which became finding 1. Findings (all code-verified before fixing):
1. BLOCKER modset corroborate_user_dir_live: logs/ junction (or symlinked entry) into the
   REAL logs dir corroborates a WRONG user dir — RM truthfully names the PID on the target.
2. BLOCKER validate_repack_manifest_modset presence-only: non-hash hash strings, arbitrary/
   EMPTY dlc arrays passed while the live resolver fails an empty census closed.
3. HIGH d47 work-without-emission sum missed advisor_roster/appointed_advisor/portrait
   rows — an advisor-only built-then-discarded frame escaped the gate ("covers ALL output
   families" comment was false).

### FIX ROUND 2 (uncommitted until its own gate SHIPs — user directive: adversarial review
BEFORE commit)
1. Canonical containment: canonicalize(user_dir/logs) must equal canonicalize(user_dir)/
   logs (OsStr case-insensitive) + per-entry file_type().is_symlink() refusal. NOT keyed on
   raw reparse attribute (cloud-sync files-on-demand placeholders are reparse points and
   OneDrive-redirected Documents is a stock end-user Windows config — those must pass on
   ANY user's machine, not just this rig). user_dir-itself-an-alias stays corroborable
   (dlc_load.json + logs then share
   one physical dir). Hardlink residual documented out of threat model. New test
   user_dir_corroboration_refuses_junction_redirected_logs (mklink /J, held handle).
2. Deep schema: 64-hex per-mod hashes, nonempty exe_path/pdx_user_dir, user_dir_source ∈
   {override, probed}, dlc arrays = nonempty strings, installed NONEMPTY, set-equality
   enabled == installed − disabled (disabled may list uninstalled). Drift guard
   live_resolved_report_json_passes_repack_validation feeds ModsetReport::as_json through
   the validator. Negative tests: fake hash, empty census, non-string dlc, inconsistent
   enabled, unknown provenance.
3. domain47_adapter_output_rows helper sums ALL 8 output families; regression test proves
   each family alone counts and *_seen/rejection/dead_roots counters don't.
Suites: recorder 197+199/0, collector 357/0, release build clean. Gate: fixround2_regate
(prompt ~/.claude/hoi4_gate_prompts/fixround2_regate.txt, reviews the UNCOMMITTED tree).
Verdict recorded here on return; commit only on SHIP.

### FIX ROUND 2 GATE VERDICT: REFUTED (tid in threads log, fixround2-regate)
Junction fix + d47 fix SURVIVED all attacks (incl. live check: the actual OneDrive user
dir canonicalizes to exactly user_dir/logs, 22 entries no symlinks; all 8 d47 families
confirmed; alias-user_dir argument accepted; hardlink residual accepted under the stated
threat model). 3 NEW BLOCKERs, all deeper validator gaps — each code-verified:
1. disabled ⊄ installed passed, but the LIVE resolver requires exact-match disabled keys
   (modset.rs G2-finding-6 arm at ~554 — round 2 inferred the rule from the enabled
   derivation alone and even blessed an uninstalled disabled entry in the fixture).
2. Identity-less mods passed: hashes without descriptor_entry/descriptor_path/
   content_path/content_fingerprint — all of which a live resolved report necessarily
   carries (resolution fails closed without them; files==0 walk fails closed too).
3. dlc_load_mtime_unix > process_start_unix passed, but live fails mtime >= start closed;
   equality must stay legal (serialization floors to seconds).

### FIX ROUND 3 (direct fixes; final EXTRA gate past the escalation cap — justified:
lossless-critical modset validation)
1. disabled.is_subset(installed) required; fixture ghost entry removed; wrong round-2
   comment corrected in place.
2. Per-mod: nonempty descriptor_entry/descriptor_path/content_path + content_fingerprint
   object with files>=1 (live invariant), bytes u64, max_mtime_unix i64. Negative tests
   for each; drift-guard test already carries all fields.
3. launch-order: dlc_load_mtime_unix > process_start_unix refuses ("postdating"),
   equality passes (same-second test). Suites: recorder 197+199/0.
Gate: fixround3_regate on the uncommitted tree. Commit only on SHIP; if REFUTED again,
findings + state land here and the round stops for orchestrator/user review (cap logic:
2 fix rounds + extra gate spent).

### FIX ROUND 3 GATE VERDICT: REFUTED (tid in threads log, fixround3-regate)
Class narrowed decisively: ZERO false refusals (pre-1970 mtimes, unicode/UNC, single-file
archives, dup mods, flooring edges all attacked and held), ZERO regressions on the
junction/d47 fixes, disabled-subset + identity checks confirmed matching live. 3 remaining
BLOCKERs, all "validator accepts live-impossible values" (verified against live code):
1. fingerprint.files unbounded above (live walk caps MOD_CONTENT_MAX_FILES=200k) +
   fingerprint.max_mtime_unix could postdate process start (live fails content mtime >=
   start closed).
2. BTreeSet collapse erased order/dup invariants: live installed census is SORTED unique
   (census .sort(), distinct paths) and enabled preserves that order; disabled is raw
   launcher input (dups/reorder legal live — must NOT reject).
3. Uppercase hex accepted (live sha256 encoder is lowercase-only); negative
   process_start_unix accepted (live query rejects pre-epoch creation); name/version/
   remote_file_id keys omissible (as_json always writes them, string-or-null).

### FIX ROUND 4 (direct; convergence judgment: findings are strictly narrowing, no
false-refusal/regression findings 2 rounds running — one more gate rather than stopping
with a dirty tree; if THAT refutes, stop and surface)
1. files <= pub MOD_CONTENT_MAX_FILES (constant made pub, shared); fingerprint mtime >
   process_start refuses, equality + negatives legal.
2. Vector semantics: installed strictly sorted/unique (windows check), enabled must equal
   order-preserved installed−disabled vector; disabled stays free-form but subset-checked.
3. Lowercase-only sha256_ok; process_start_unix >= 0 (dlc_load mtime may be negative —
   NTFS pre-1970); name/version/remote_file_id must be present string-or-null.
Tests: 9 new negatives (uppercase, neg start, missing name, over-cap files, content
postdating + same-second pass, dup installed, reordered enabled, dup disabled PASSES);
drift guard gains a None-identity mod pinning the null path. recorder 197+199/0.
Gate: fixround4_regate. Commit on SHIP; on REFUTED stop + surface to user.

### FIX ROUND 4 GATE VERDICT: REFUTED (tid in threads log, fixround4-regate)
Stop-tripwire evaluated and OVERRIDDEN with cause: finding 3 (closed objects) makes the
accepted grammar exactly the serializer's output, TERMINATING the looser-than-as_json class
by construction — round 5 is the structurally last round, not gate fatigue. Gate showed
judgment (explicitly declined sub-bar findings: filename chars, extreme timestamps).
Findings (live-verified): 1. free-form DLC names passed (census emits ONLY
dlc/<dir>/<name>.dlc — my own fixtures used live-impossible names); 2. descriptor_path
unconstrained vs descriptor_entry (live derives path = absolute-entry-as-is OR
user_dir.join(entry)); 3. unknown keys accepted at all 5 object levels (as_json is closed);
4. LOW: accept-side boundaries unpinned (start=0, files=cap, negative mtimes, reordered
multi-disabled), drift guard mislabeled as live coverage.
Attacks that failed: census sort/order confirmed, Vec/BTreeSet Ord identical incl unicode,
200k boundary exact both sides, lowercase exact, Vec rewrite preserved semantics.

### FIX ROUND 5 (terminal by construction — the grammar is now CLOSED)
1. installed entries must be census-shaped dlc/<dir>/<name .dlc-ext> (3 components,
   forward slashes, no backslashes, extension ASCII-case-insensitive); disabled/enabled
   constrained transitively. All fixtures migrated to census-shaped names.
2. descriptor_path must equal the resolver's derivation (absolute entry as-is, else
   pdx_user_dir join; Path COMPONENT equality — the live join mixes separators).
   Negative (foreign path) + positive (absolute entry) tests.
3. check_keys closure rejects unexpected keys at root/game/mod-entry/fingerprint/dlc;
   5-level table-driven negative tests; mod entries must be objects.
4. Accept-side boundary test (start=0 + mtime -1 + files==cap + fingerprint mtime -10),
   reordered multi-item disabled passes, drift guard relabeled serializer-shape.
recorder 197+199/0. Gate: fixround5_regate (terminal). Commit on SHIP; on any non-trivial
REFUTED: hard stop, surface, no further rounds.

### FIX ROUND 5 GATE VERDICT: SHIP (tid in threads log, fixround5-regate) — CHAIN CLOSED
"No findings above the silent-loss/fabrication bar." Grammar confirmed structurally closed
at all 5 levels; every as_json key required + type/range checked. The Path::eq false-refusal
suspect REFUTED against rust-lang source (component iterators, not bytewise OsStr; join
inserts MAIN_SEPARATOR so the mixed-separator fixture is the real live shape). All
false-refusal attacks failed: unicode DLC names, uppercase .DLC, dotted dirs, UNC absolute
entries, trailing-separator user dirs, dup/reordered disabled, files==cap, start==0,
negative mtimes. Junction containment + d47 8-family sum re-confirmed intact.
Below-bar notes (accepted residuals, recorded): component-equivalent path spellings accepted
(same-file binding); unresolved branch's partial_evidence stays open-keyed (cannot
masquerade as resolved); shape gate ≠ disk authenticity (authenticated evidence is out of
scope); no non-object mod-entry fixture (rejection branch is direct + compiled).
FULL CHAIN: fixround regate REFUTED(3) -> r2 fixes -> gate REFUTED(3) -> r3 fixes -> gate
REFUTED(3) -> r4 fixes -> gate REFUTED(3+LOW) -> r5 fixes -> gate SHIP. Every hunk in the
committed diff was adversarially reviewed pre-commit (user directive 2026-07-12).
COMMITTED: e7bce36b1 (+ PRODUCT_SESSION_SEMANTICS.md, the owner's session-semantics vision
of record: dual-order auto-attach, campaign identity, later-supersedes stitching policy —
both owner decisions 2026-07-12).

### POST-CHAIN DISPATCHES (2026-07-12)
- **O5 modset gate (e9f21a1c8): SUPERSEDED, never dispatching** — its entire surface
  (fail-closed resolution, census-vs-dlc_load key format, manifest semantics) was covered
  at newer tree states by G2-regen (5B+2M, fixed) + this five-gate chain (SHIP).
- **G1 final net gate resumed** (tid 019f4ebb-0fb6-7cc1-88e8-3755a1ae6b2f, g1_prompt.txt
  piped with tree-advanced preamble): the 6-commit d47-dedup/d69-tombstone/O7 net-state
  adjudication.
- **G4 O8 design gate resumed** (tid 019f4f8d-2021-7ca3-9b8d-1ac319ff970a, full prompt
  re-sent — first turn died pre-consumption on the limit).
Remaining after these: C0 census tooling gate (3ecf9f7eb) + F5 container/fold/B3
integrated final gate (owns the final word per plan §5; prefetch 16->8 batches in).

### G1 + G4 RESUME VERDICTS (2026-07-12): both DISCHARGED AT HEAD — stale evidence
Both resumed threads reasoned over their INLINED evidence (tree 319dc641b, tool calls
prohibited by the broken-shell-era prompts) and re-derived verdicts already processed:
- **G1 net gate REFUTED(3 CRITICAL)** — ALL THREE pre-fixed at HEAD with their own citing
  comments + re-gates: d43 PARTIAL-from-first-incomplete-hold (lib.rs ~29475, landed in
  the G1res batch), d69 fail-closed-on-any-stable-vtable-mismatch (lib.rs ~37960, re-gate
  019f5066), d47 country-disagreement quarantine + leader-ptr tombstone (admit_character
  ~18508, review 019f53fb). VALUE RETAINED: its failed-attacks list discharges the OWED
  adjudication of ebdc5bc4b + 1fbd9f3a4 (a679df5b8 amendment) — stable zero/high-bit d69
  members confirmed structural non-members, lossy-adoption/degraded-bypass/O7/_sample
  surfaces clean, no new findings. Owed-gate debt CLOSED.
- **G4 O8 design REFUTED(9)** — all nine corrective amendments were ALREADY FOLDED into
  CONFIRMED_COMPLETE_DESIGN.md as the design of record on 2026-07-11 (0ece437f3, same tid
  019f4f8d): deny-by-default declaration registry (empty tolerance set), transactional
  epochs (session nonce + epoch tokens), mandatory presence sentinel, per-subsource
  evidence units over aggregate bits (= C1 stage A, in flight), Captured =
  contract-completion, wire major bump, C0 = discovery-only, provenance axis + d17
  NATIVE default. C1 stage 3 tolerance stays blocked behind stages B-E as amended.
AUDIT BLOCKING LIST now exactly: C0 census tooling gate + F5 integrated final gate.

### C0 TOOLING GATE VERDICT: REFUTED 5 CRITICAL + 1 HIGH + 1 LOW (tid in threads log,
c0-tooling-gate) — the v1 census was itself a false-green instrument
1. no roster/coverage check (a fully dark domain = invisible, exit 0; registry actually
   has 56 schedulable domains — d18 country_identity is ActiveIntegration = DOCUMENTED
   profile exclusion, verified in status_in_production_profile);
2. empty/unreadable/gate-failed inputs pass (zero sessions = exit 0; decodable/ordered/
   domain_coverage.ok/hourly ignored);
3. problems + fold state never gate (world_folded is STATIC FOLDED_DOMAIN_IDS membership,
   not per-session acceptance — verified main.rs:17520);
4. not domain-aware: d49/d51 lane-local bits misread as global; bit13 on d45 (an INVALID
   combination present in the built-ins) silently accepted; unknown bits >13 dropped by
   bits_of();
5. d69 must-be-zero policy documented but dead (key "tech_sharing_groups" vs recorder's
   "tech_sharing_groups_live");
6. session-OR attribution launders second causes (ATTRIBUTED cleared the whole pair);
7. LOW mechanicals. Failed attacks: recorder field names exact; problems-object iteration
   fine; real d69 regression currently exits 1 (via the misspell, ironically).

### C0 CENSUS v2 SHIPPED (uncommitted until the combined gate SHIPs)
Full rewrite: pinned 60-entry roster (55 profile) + drift-blocking both directions;
schema/stream gates (decodable/ordered/coverage/expected-set/parse/ordering/non-bit-derived
hourly problems; zero-usable-manifests blocks); dark-domain + zero-data blocking;
fold problems block; world_folded renamed-in-semantics to fold_supported (False = drift);
domain-aware bits incl. d53 lane bits 16-22 (domain44_compact.rs) + d60 bit 16
(capwar compact.rs) — BOTH previously invisible; unknown-bit blocking; bit13 valid only
on d17/d53; ATTRIBUTED policies must_be_zero (d69 tripwire ENFORCED, correct key) and
candidate (labels, still blocks pending pass-2 status_origins reconciliation);
--selftest pins all gate false-green scenarios (11/11). ASCII-only output (end-user
codepages). LIVE RESULT on the 2 production manifests: exit 1 with 41 blocking findings =
the HONEST open C0 state (v1 said 13); newly visible: d53 RESISTED/UNVALIDATED segment
badges (16/17/20/22), d45 bit13 invalid-combination anomaly (needs attribution: who sets
it?), d48 fold problems (skipped construction rows reaching the fold), d51 lane badges.

### STAGE-A RESIDUE GATE VERDICT: REFUTED 1 MEDIUM (tid in threads log,
stagea-residue-gate) + FIX
Finding: the d48 packed multi-cause sample under LAST-VALUE semantics kept only the final
frame's causes (hour-100 country-root + hour-101 resource-row left no country-arm trace).
Failed attacks: id/name alignment, predicate match, capture-semantics drift (additions
only), no double-recording, no missed d65 pre-walk returns, recorder so_* generic.
FIX: one origin id PER CAUSE (SO_D48_LANE_* x7, ids 35-41, COUNT 42); collector maps the
7 pub CollectorStats counters via domain48_lane_partial_causes(); test covers per-cause
mapping + the gate's two-frame different-causes scenario. collector 358/0, contract 29/0.
Combined pre-commit gate covers stage-A residue v2 + census v2 together.

### COMBINED GATE VERDICT: REFUTED 4 CRITICAL + 2 MEDIUM (tid in threads log,
combined-stagea-census) — part 1 runtime SOUND (recorder aggregation verified end-to-end,
d48/d65 attribution complete, no capture drift); all criticals on census v2:
1. pinned roster never checked against the registry SOURCE (pin+old-manifests can agree
   while the registry moves); 2. hourly_requirement optional/met-false tolerated (the
   authoritative verification gate requires required+met); 3. bool-passes-isinstance-int
   + negative counts + nonempty coverage errors + unvalidated session_id; 4. duplicate
   JSON members last-wins (clean dup shadows a failing first member). MEDIUMs: 5. id/name
   array order not mechanically coupled (swap compiles + passes uniqueness test);
   6. selftest bypassed the real entry path. BONUS: d45 bit13 adjudicated REAL (d45 ORs a
   dependency bundle's flags at collector lib.rs ~39359 — d17-only bit bleeds; census
   correctly exposes it); roster pin verified exact vs all 60 descriptors.

### FIX ROUND (v3 census + coupling test; uncommitted pending gate round 3)
1. registry_drift(): startup cross-check of the pin against compact_contract/src/lib.rs
   (regex over domain(...) entries; parse shortfall = drift = blocking BEFORE manifests).
2. hourly must positively prove {required: true, met: true, problems: []} — absent/
   older-schema/met-false block (live manifests now honestly add 2 hourly blockers).
3. strict types: type(v) is int (bools rejected), nonneg counters, exact-[] coverage
   errors, string-list expected_domains, nonempty unique session_id.
4. json object_pairs_hook rejects duplicate members recursively.
5. Rust test every_id_constant_indexes_its_exact_unit_name pins the FULL 42-entry
   id->name registry (contract 30/0).
6. selftest rebuilt through the REAL path (evaluate_source/run_paths): 25/25 incl.
   registry-drift, registry-exact, dup-member, dup-session, hourly x3, strict-type x5,
   d53/d60 lane bits, zero-usable. d45 bit13 now ATTRIBUTED candidate (real bleed, fix =
   mask dependency bits — new work item). Live census: exit 1, 47 blockers (45 + 2
   hourly) = the honest C0 state.

### ROUND-3 GATE VERDICT: REFUTED 2 CRITICAL + 2 MEDIUM (tid in threads log,
combined-round3) — all census; stage-A runtime held again; gate EXECUTED its probes
(block-commented an entry in memory, watched the pin stay silent):
1. registry parse comment-blind + prefix-arity (a /*...*/'d active entry produced no
   drift; trailing-arg changes invisible); 2. absent parse/ordering evidence exited
   green ("!= [] and is not None" tolerated absence); 3. float expected_domain_count +
   dup-padded expected_domains passed via set collapse; 4. selftest never pinned main()'s
   decisive exit return (mutating 1->0 in memory kept 25/25 green).
Failed attacks: hourly shapes, strict per-domain types, recursive dup-key hook (nested
dup members raise), 42-entry Rust id/name pin unforgeable, reordered registry
order-free, stage-A unchanged-sound.

### ROUND-3 FIX (v3.1, uncommitted pending round-4 gate)
1. registry_entries(): scoped to the pub const ASSIGNED_COMPACT_DOMAINS decl,
   comments stripped BEFORE locating the `];` terminator (own finding on top of the
   gate's: a comment at lib.rs:1268 contains a literal `0x3316E90];` which truncated the
   raw-text scope at 58/60 entries — comment-first ordering is load-bearing), full 7-arg
   arity through the closing paren, requires_data_record pinned (clock/d59/frame false,
   all non-profile — a flag change re-judges the dark-domain check). Parse shortfall =
   drift = blocking.
2. parse/ordering_errors must be EXACTLY [] (absent/null/mistyped blocks).
3. expected_domain_count strict int; expected_domains length + set equality (dup-padding
   dead).
4. selftest pins main()'s own exit codes via tempfile manifests (clean->0, blocking->1,
   registry-unreadable->1) + registry-commented-entry and registry-extra-arg drift cases.
Selftest 34/34; live census exit 1 / 47 blockers unchanged. Gate: combined-round4.

### ROUND-4 GATE VERDICT: REFUTED 1 CRITICAL + 1 MEDIUM (tid in threads log,
combined-round4; executed probes again):
1. regex comment-strip vs rustc: NESTED /* /* */ */ left a hidden entry visible (no
   drift); a block comment OPENED BEFORE the decl made raw find select the commented
   corpse while an appended active copy (d69->70) went unjudged.
2. process exit unpinned: gate dropped the sys.exit wrapper in memory — 47 blockers
   printed, process exited 0, selftest stayed 34/34 (main() was called in-process only).
Failed attacks: string-`];` and alias-`]` truncations both BLOCK (fail-closed), extra-arg
+ plain-commented-entry block, top-level parse/ordering precedence correct + authoritative,
float count/dup-padding/absent-evidence block, drift-early-return + zero-usable mutations
caught by selftest.

### ROUND-4 FIX (v3.2, uncommitted pending round-5 gate)
1. strip_rust_comments(): character-level lexer — NESTED block comments, line comments,
   string/char literals with escapes, lifetimes ('static is not a char literal); comments
   blanked to spaces (LENGTH-PRESERVING, so stripped offsets are raw offsets); decl located
   in STRIPPED text and must appear EXACTLY once (0 or >1 = drift). String-`];` truncation
   stays fail-closed by design.
2. selftest gained registry-nested-comment + registry-shadowed-decl probes (the latter
   needed the length-preserving offset trick — a raw `];` find hits the lib.rs:1268
   comment) and PROCESS-level subprocess pins (clean rc=0, blocking rc=1) that catch the
   dropped-sys.exit mutation.
Selftest 38/38; live exit 1 / 47 blockers. Gate: combined-round5 (expected terminal).

### ROUND-5 GATE VERDICT: REFUTED 1 CRITICAL (tid in threads log, combined-round5) —
round-4 findings confirmed closed; lexer survived raw/byte strings, escapes, lifetimes,
comment-semantics agreement, shadowed decls, string-embedded decls. The ONE fail-open
left: #[cfg(...)] on a registry entry — release omits it, test builds (and the Rust
exact-ID test) keep it, textual pin sees it -> no drift, stale manifests agree. Gate
correctly separated this from the fail-closed raw-string residual.

### ROUND-5 FIX (v3.3, uncommitted pending round-6 gate)
registry_scope(): any `#` outside string literals inside the array = parse shortfall =
fail closed ("conditional compilation is not textually evaluable"); distinct drift
message; 2 new selftest probes (#[cfg(test)], #[cfg(debug_assertions)]). Selftest 40/40;
live exit 1 / 47. CLOSURE ARGUMENT for round 6: every remaining non-literal construct
(macro invocations, include!, const indirection) fails the entry regex -> count/name
drift -> fail closed BY CONSTRUCTION; cfg was the unique textual-match-compile-different
case. Gate: combined-round6 (terminal by that argument).

### 2026-07-12 ~21:30 — CREDIT WALL #4 + D04 STAGE-A CAPTURE HARVEST
Round-6 attempts a/b/c all wedged on the host sandbox; round6d (fresh, tid
019f5611-aac2...) died instantly on the ChatGPT usage limit (reset stated: Jul 13
00:50). QUEUED post-reset via relaunch_post_reset_20260713.sh (~/hoi4_lanes/):
round6gate (-> round6e, CRITICAL PATH for committing census v3.3 + stage-A) +
locjoin fresh + speedpause fresh. Long-running lane sessions from before the wall
left alive (they retry internally).

CAPTURE production_speed5_ShortSpeed5_d04stageA_211128 (600s, paused D04, stage-A
DLL b6205df4) — LIVE EVIDENCE FOR THE UNCOMMITTED BATCH:
- d69 tech_sharing_groups_live: status_flags_or=0 (CLEAN), 3 groups / 17 members,
  world_folded=true — vacant-slot fix holds on the cold baseline frame.
- d68 session_header_live: flags_or=0, 1 row, folded — campaign-identity source
  (game_unique_id/seed) reads clean; corroborates CAMPAIGN_IDENTITY_DESIGN.md.
- STAGE-A STATUS-ORIGIN PRODUCERS EMIT LIVE (new ids 33-41):
  so_d48_lane_skipped_construction_rows value=1 sample=11;
  so_d65_partial_dropped_ambiguous value=1 sample=5 — both ride the stream into the
  manifest ledger with first/last_hour stamps. Also live: d43 empty_cold, d45
  partial_targeted_timed_structural, d53/d60 source_unavailable (+samples), d54
  partial_skipped_rows.
- Pause semantics confirmed: sweep consumers d43/d47/d53/d60 flags 256/768 =
  cold-state SOURCE_UNAVAILABLE family, NOT regressions. d51 flags_or=14336 =
  exactly bits 11-13 (the w12-d51-badges attribution targets) — good fixture.
- extraction_perf_gate=false + .failed.battleplan suffix: expected on a paused
  capture (partial_data_seen from the cold sweep consumers).
NOTE: this capture is VALIDATION EVIDENCE, not the adversarial gate — round6e still
owed before commit.

LANE LEDGER DELTA: w12-d53-badges COMPLETE (dossier committed f40229d7e in-worktree;
verdict: all 4 badges chronic capture gaps 717/717 frames, no safe in-fence clear,
proposed diffs in dossier; census --selftest false-green claim needs verification —
flagged for the round6e reviewer follow-up). w12b-events-decisions-scope COMPLETE
(dossier e07fef384; fired-events full A18 needs runtime hooks, d16/d45 partial folds
possible now, d60 lacks warscore/IC/wargoals). w12b-speedpause-scope BAILED (wedge;
provisional intel: speed NOT read anywhere today, d57 = pause rows only, no speed
offset in build profile). w12b-locjoin-verify: 40-try refire loop exhausted into the
wall; requeued.

ADJUDICATION (orchestrator-verified, 2026-07-12 late): w12-d53-badges' census
"--selftest falsely exits 0" finding reproduced against the OLD census at the lane's
worktree base 6fd03c653 (treats the flag as a manifest arg -> 0 domains -> GREEN exit
0 = the known v1 empty-input fail-open). The uncommitted v3.3 already closes it:
--selftest is a real entry (40/40) and zero-usable input blocks (exit 1, selftest
zero-usable: blocking=2). No new work; closure rides the round6e-gated commit.

LANE LEDGER DELTA 2: w12-pass2-spans COMPLETE (08005fe7a: c0_pass2_spans.py selftest
6/6, both production streams byte-exact, zero so_* pre-stage-A = correct producer
tracking; stale-base claims re-adjudicated same as d53-badges'). w12-d48-fold-skip
COMPLETE (code lane, in-worktree commit: non-59 queue entries reclassified
informational filter — new filtered_construction_non_construction_line_rows counter,
legacy counter kept always-zero; 26/0 tests). MERGE-TIME RECONCILIATION ITEM: the
uncommitted stage-A domain48_lane_partial_causes() maps the LEGACY counter to an SO
id — harmless post-merge (always zero) but the mapping should drop-or-remap to the
new filtered counter as INFORMATIONAL (not a partial cause) in the merge commit, and
the census d48 expectations re-checked. The D04 capture's so_d48 sample=11 likely
decomposes into non-59 filters (would clear d48 PARTIAL) — verify on the first
post-merge capture.
