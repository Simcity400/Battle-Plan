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
| 6 | d43/d69 chronic-flag attribution (SOURCE/PARTIAL while data flows) | A→C | truthful gates | **QUEUED for resume** — dispatched 2026-07-10 21:4x, Codex hit usage limit mid-turn (retry ~21:48). Thread `019f4bd7-0181-7d33-b2f9-74c7495a5e50`; resume via `codex exec -s read-only ... resume 019f4bd7-...` (never re-dispatch fresh). Prompt logged in `~/.claude/codex_threads_hoi4.txt` |
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
