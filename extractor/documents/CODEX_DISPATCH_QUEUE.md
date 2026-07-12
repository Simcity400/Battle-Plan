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
