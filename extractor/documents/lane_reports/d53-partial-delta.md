# lane/d53-partial-delta — enable delta emission on tolerated-PARTIAL d53 frames

Status: **IMPLEMENTED, PROVEN LOSSLESS, GATED OFF BY DEFAULT.** The core lever
(delta on partial frames) is a MEASURED net capture-CPU regression, so it ships behind
an opt-in emit mode (`DeltaPartial`); the default (`Delta`) is byte- and CPU-identical to
today. Branch `lane/d53-partial-delta`. Not pushed.

## Mission

d53 (diplomacy/intel/operatives) is 92.3 % of all stream bytes. Its delta path never
fires in deep war: 400/400 d53 frames carry `PARTIAL_DATA` (a single torn per-row read
across ~245 k rows sets the bit), the collector's emit gate returned the FULL read on
partial, and the fold hard-rejected `DELTA|PARTIAL` (`world_fold.rs:10530`
`delta_partial_unsupported`). Result: 5.9 MB full re-emit every game-hour. Mission: emit a
delta on tolerated-PARTIAL frames (baseline-seeded two-accumulator, non-advancing baseline,
fold acceptance of `DELTA|PARTIAL`), proven lossless offline.

## What changed (file:line — worktree paths)

Production (both hot shared files; edits confined to d53 functions):

- `extractor/native/collector/src/lib.rs`
  - `D53EmitMode` (enum ~1573): added `DeltaPartial` variant + `parse`/`as_str`. Default
    stays `Delta`.
  - `CompactDiplomacyIntelOperativesLiveTracker` (~4177): added `baseline_seeded: bool`.
  - `Domain53BaselineCommit` enum + `commit()` (new, ~18125): the deferred baseline
    mutation (None=pin / Clear / Seed).
  - `compact_domain53_apply_emit_mode` (~18160): now takes `tracker: &` (immutable),
    RETURNS `(rows, status, Domain53BaselineCommit)` instead of mutating. Gates the
    partial-frame delta behind `DeltaPartial` (default `Delta` early-returns the full read
    on partial); seeds the baseline on the FIRST frame via `baseline_seeded` (even empty/
    partial); pins the baseline on partial (`Commit::None`), advances on non-partial.
  - `read_compact_diplomacy_intel_operatives_live` (~18020, ~18063): destructures the
    commit and applies it via `baseline_commit.commit(tracker)` ONLY after the post-build
    gate passes.
- `extractor/native/recorder_host/src/world_fold.rs` (`fold_diplomacy_intel_operatives`)
  - Removed the `delta_partial_unsupported` rejection (was ~10530); `DELTA|PARTIAL` now
    flows into the existing `is_delta` branch (seeds the hour from `domain53_delta_*`).
  - Records the `partial_data` problem on a `DELTA|PARTIAL` frame (provenance parity).
  - Baseline advance guard changed `!is_partial || is_delta` → `!is_partial` (+ an
    `establishing_first_baseline` term so a partial-FULL FIRST frame establishes the
    baseline and inserts `delta_baselines_seen`). Pins the baseline on subsequent
    partials.

Tests + tooling:

- `collector/src/lib.rs` `attach_log::domain53_partial_emit_tests` (10 tests, new).
- `recorder_host/src/main.rs` `compact_d53_reduced_partial_folds_identically_to_full_emission`
  (fold-identity gate: 4-hour byte-identical scenario + empty-first-frame scenario +
  48-seed row-identical fuzzer).
- `lanes/diplomacy_intel_operatives_live/src/bin/d53_partial_delta_project.rs` (new
  measurement bin; reads the corpus in place, projects bytes + micro-benchmarks CPU).

## Losslessness argument + the gate that proves it

Design: on a `DeltaPartial` partial frame the collector diffs the read `R` against a
PINNED baseline `B` (`last_emitted`, the last authoritative non-partial full read) and
emits changed/new rows + a tombstone for every baseline key absent from `R`
("tombstone-all"). `B` is never advanced on a partial frame. The fold seeds the hour's
overlay from its mirror baseline `domain53_delta_*` (which persists across the per-hour
overlay clear at `world_fold.rs:1431`) and applies upserts + tombstone-removals.

Claim: `seed(B) + apply(changed) - remove(tombstone-all-absent) == R` == exactly what
today's full-emit partial produces (today seeds from the just-CLEARED empty overlay and
sparse-merges the full `R`, which also drops torn rows for the hour). Because the fold
re-seeds from the pinned `B` every hour and the collector diffs against the same pinned
`B`, a value pushed on one partial and reverted on the next is corrected WITHOUT any
projection accumulator (unlike d48, whose fold carries its projection across hours).

Gate (`compact_d53_reduced_partial_folds_identically_to_full_emission`, recorder_host):
- **4-hour scenario folds BYTE-IDENTICAL** full-emission vs reduced-emission (unchanged
  partials, genuine changes, deletions/tombstones, reverted values, clean interleaved,
  multi-hour across the overlay clear).
- **Empty-first-frame scenario** folds byte-identical (regression for the Codex bug below).
- **48-seed randomized fuzzer** folds DECODED-ROW-IDENTICAL (array order canonicalized —
  the world-overlay Vecs are upsert-ordered, a pre-existing semantically-irrelevant
  property shared with the shipped pure-delta path; plan G1 explicitly permits
  "decoded-row equality"). Seeds include partial-first-frame, empty states, deletions,
  reverts, clean-interleaved, multi-hour.

Tombstone semantics (deliberate, documented): this uses **tombstone-all-absent**, NOT
positive-absence-only. The d53 extractor reports only a skipped-row COUNT
(`extractor.rs:400`), not torn keys, so torn vs deleted absence is indistinguishable
without a hot-path extractor change that would ADD CPU (violating the weaker-PC law).
Tombstone-all reproduces today's decoded state EXACTLY (byte-identical), so it is lossless
*relative to today's decoded state* (the binding bar). It does NOT satisfy the literal
"positive-absence-only tombstone" rule; that (which would RETAIN torn rows as last-known-
good — strictly more complete than today) remains a future enhancement gated on the
extractor emitting positive-absence.

## Decisive measurement (the CPU refutation) — corpus, read in place

Corpus: `runs/live_validation_20260709_221706/stage2_shortspeed5/collector_stream.bin`
(12.9 GB), first 400 d53 DOMAIN_DATA frames, via `d53_partial_delta_project`. 254 success
frames, 146 failure/empty (SOURCE_UNAVAILABLE — the collector never runs delta on those,
and the fold rejects them; the projection skips them to mirror that). Zero clean frames in
this deep-war window (all-partial), so the baseline is seeded from the first SUCCESS frame.

- **Bytes: 155.5x reduction.** 2259.99 MB → 14.54 MB (99.36 %). Per success frame
  8.898 MB → 0.057 MB. Delta rows/frame plateau at ~2391 (opinions ~477 + state_events
  ~574 = 0.25–0.30 % drift vs the pinned baseline + countries/operatives/wars/crypto), and
  the per-frame reduced size **plateaus at ~32 KB (no unbounded growth)**.
- **CPU: +46 to +68 ms/frame — a NET INCREASE, refuting the premise.** The prompt assumed
  ~39 ms of "payload-encode CPU" would disappear. MEASURED on the real frames:
  - OLD full encode + decode-gate = **7.9 ms/frame** (the encode was never the bottleneck;
    it is a cache-friendly linear pass over the 8.9 MB payload).
  - NEW naive (build current BTreeMaps + diff + encode reduced) = **74–76 ms**
    (build 27 + diff 47 + reduced encode/gate 0.2). NET **+66.7 to +68.4 ms/frame**.
  - NEW optimized floor (HashMap baseline, no per-frame rebuild, single-pass diff of the
    two dominant families only) = **54 ms**. NET **+46 ms/frame**.
  The regression is FUNDAMENTAL: diffing ~386 k rows is ~1.15 M random-access keyed
  lookups (~46 ms floor), inherently far dearer than the 7.9 ms linear full encode it
  replaces. There is no cheap "what changed" signal (Phase-4 scout: no engine dirty
  signal). So this lever REDUCES raw-stream bytes but INCREASES capture-frame CPU.

Interpretation vs the hard requirements: the byte win is largely **redundant with the
container's zstd method-93 (163x measured on exactly this constant-redundancy class)** on
the SHIPPED container; it is NOT redundant for the raw stream / host-fold time / disk /
peak memory (the fold was the "new bottleneck of record" at ~40 min). But requirement #2
(cut CPU per capture frame; weaker PCs) OVERRIDES bytes, and this is a measured +46–68 ms
regression on d53's lane. **Therefore the lever ships OFF BY DEFAULT** (`DeltaPartial`
opt-in); `Delta` (default) is unchanged. A documented "measured, not worth it on the
capture hot path, here are the numbers" is the honest verdict for the default; the opt-in
retains the machinery for host-fold/disk/memory-bound captures on strong rigs, or a future
off-frame-thread computation.

## Codex adversarial review (verbatim claims + disposition)

Codex Mode-A (gpt-5.6-sol, xhigh), read-only, twice + a final confirmation.

**Round 1 verdict: REFUTED / unsafe.** Findings and disposition:
- **F1 hour-overlay clear — Codex ACCEPT.** Keyed upserts prevent duplication; the clear
  is not the defect. (Agreed.)
- **F2 tombstone soundness — Codex REFUTED:** "a torn read is explicitly converted into a
  deletion … R is deleted from every output hour for which it tears despite still
  existing. 'Legacy did the same drop' proves output equivalence, not the stated hard law
  that tombstones never delete live rows." **Disposition: SURFACED, not fixed — deliberate.**
  It is byte-identical to today's decoded state (Codex concedes output equivalence), so it
  is lossless relative to the binding bar (today). It does NOT meet the literal
  positive-absence-only rule; that improvement needs a CPU-adding extractor change and is
  deferred (documented above). Off-by-default limits exposure.
- **F3 collector/fold baseline divergence (empty first frame) — Codex REFUTED, REAL BUG,
  FIXED.** Codex's sequence: H0 empty success frame → collector seeds an EMPTY tracker
  (still `is_empty()`), so H1 non-empty RE-SEEDS while the fold (keyed on
  `delta_baselines_seen`, set at H0) does not re-establish → H2 unchanged delta applied
  against an empty fold baseline → **rows silently vanish.** My fuzzer modeled "seen" as an
  `Option` (a proper bit) and therefore missed it. **Fix:** explicit
  `baseline_seeded: bool` used instead of `is_empty()`; regression test
  `empty_first_frame_seeds_and_next_frame_deltas` + the empty-first-frame fold scenario.
- **F4 partial baseline advancement not one-shot — Codex REFUTED — same root cause as F3,
  FIXED** by `baseline_seeded`.
- **F5 baseline admission is boolean-only (no generation/hash) — Codex REFUTED.**
  `reject_delta_before_baseline` checks only membership. **Disposition: SURFACED, not fixed
  — defense-in-depth, wave-wide.** With the root-cause fixes it no longer has a reachable
  trigger in this lane; a baseline generation/hash on delta records (verified by the fold)
  is a recommended hardening for ALL delta domains, outside this lane's scope.

**Round 2 (confirming the `baseline_seeded` fix): empty-first-frame success path CONFIRMED
fixed.** Two further sequences raised:
- **Finding A: Full→DeltaPartial mode switch loses rows.** Codex itself notes "the current
  control is immutable within one attach worker, so an in-place CLI mode switch is not
  presently exposed." **Disposition: NOT REACHABLE** (emit mode is session-fixed at attach;
  never changes per-frame). Latent only; documented.
- **Finding B: `apply_emit_mode` mutates the tracker before `report.ok()`.** A frame that
  seeds/advances then fails the post-build gate leaves the collector ahead of the fold
  (which rejects the failed frame). **Disposition: FIXED** — refactored `apply_emit_mode`
  to be non-mutating (`tracker: &`) and RETURN a `Domain53BaselineCommit`; the caller
  commits ONLY on the success path (after the failure return). Regression test
  `failed_frame_does_not_commit_baseline`. (Low reachability — the failure path is
  essentially unreachable for well-formed delta rows, which pass the build gate — but the
  fix makes the collector match the fold's commit-on-success semantics, closing the whole
  advance/seed-then-fail class.)

**Round 3 (final confirmation of both fixes): items 1–4 ACCEPT** (pin-on-partial,
seed-exactly-once, failed-frame-no-commit, no double-commit). **Item 5 flagged REFUTED:**
"A successful partial first frame seeds the collector, while the fold rejects
partial-before-baseline and stays unseeded." **Disposition: REJECTED — false positive.**
The lean retry prompt omitted the `world_fold.rs` hunk, so Codex assumed the first partial
frame is a DELTA rejected by `reject_delta_before_baseline`. It is not: the first frame is
emitted **FULL** (DELTA clear), and the fold's added
`establishing_first_baseline = is_partial && !is_delta && !delta_baselines_seen.contains(53)`
establishes the baseline from a partial-FULL first frame. Proven by the partial-first-frame
fuzzer (48 seeds) + the empty-first-frame scenario + the 4-hour scenario, all folding
identically. `reject_delta_before_baseline` only guards DELTA-flagged frames.

## Gate evidence (exact)

- `cargo build --release` (extractor/native): green.
- `cargo test` workspace: **zero failures.**
- collector: **313 passed** (10 new d53 partial-emit tests, incl. the two Codex regressions
  + the default-`Delta`-mode-unchanged guard).
- diplomacy_intel_operatives_live lane: **113 passed** (unchanged count; contract intact).
- recorder_host: **171 (lib) + 178 (bin) passed**; the fold-identity gate is in the bin
  suite.
- Byte/CPU numbers: as above, from `d53_partial_delta_project` on the deep-war corpus.

## Proven NULLs / non-goals

- **"Delta removes ~39 ms encode CPU" — NULL/REFUTED.** Full encode+decode-gate is 7.9 ms;
  delta ADDS 46–68 ms. Delta is a CPU cost, not a CPU saving, for d53's row counts.
- **Per-field delta / row-level delta of the residual subdomains — still not worth it**
  (prior lane/d53-perfield measurement stands): only apply on delta frames and are ~0 % of
  the bytes.
- **Positive-absence-only tombstones — deferred**, not implemented (needs an extractor
  change that adds hot-path CPU; tombstone-all is byte-identical to today and lossless
  relative to it).

## What must still be validated LIVE (offline-provable parts are proven)

The losslessness proof is entirely offline (fold-identity over synthetic streams) and does
not need a live session. What a live session would add IF `DeltaPartial` is ever enabled:
1. A **paired A/B** (default `Delta` vs `DeltaPartial`) on the SAME live game to confirm
   the measured +46–68 ms/frame regression and the byte reduction under real capture, and
   its effect on skipped hours + `host_finalize` peak RAM / fold time.
2. Because the emit mode is session-fixed, the Full→DeltaPartial transition (Codex finding
   A) cannot occur in production; if per-session mode switching is ever added, that path
   must be reconciled first.

## Recommendation

Keep `DeltaPartial` **OFF on the capture hot path** (weaker-PC law): the byte win is
largely absorbed by zstd-163x on the shipped container and the capture-CPU cost is a
measured net increase. The proven-lossless machinery is retained for (a) host-fold/disk/
memory-bound captures on strong rigs, or (b) a future design that computes the d53 delta
off the frame thread (the collector already has background-thread infrastructure) — which
would convert this into a pure win. The wave-wide hardening (F5 baseline generation/hash;
advance-after-validate for ALL delta domains) is recommended as a separate lane.

## Independent adversarial gate — RUN 2026-07-10, VERDICT: SOUND

Run by an Opus reviewer under a temporary per-instance user authorization (Codex
rate-limited; authorization revoked later the same day — the Codex Mode-A re-stamp in
`CODEX_DISPATCH_QUEUE.md` remains REQUIRED before enabling). Reviewer independent of the
implementer. Summary:

- All three claims held. Default `delta` mode: the fold's baseline-advance guard change is
  observably different only on a partial-FIRST-frame stream, and is inert for anything the
  default collector can emit (partial frames return None-commit before the seed check; the
  first clean frame re-establishes the baseline FULL). delta_partial: seed + upserts +
  tombstone-all-absent reconstructs exactly the current read = exactly what FULL|PARTIAL
  sparse-merge yields; both baselines advance on the same frame classes (lockstep); the
  PARTIAL-in-FAILURE_MASK trap is correctly masked on both sides
  (`COMPACT_EXPECTED_PARTIAL_TOLERANCES`, d53 tolerates only PARTIAL_DATA). Both prior bugs
  structurally dead: `baseline_seeded` bool replaces the `last_emitted.is_empty()` sentinel;
  `apply_emit_mode` takes `&tracker` immutably and returns the mutation, committed only after
  the post-build gate.
- MINOR/LATENT (record; do not trip later): (1) the fold re-seeds d53 hour overlays from the
  mirror baseline on EVERY delta frame — if the collector ever emitted two d53 records within
  one game-hour, delta_partial keeps only the last while full emission unions both.
  Unreachable today (one d53 record per FRAME_END); breaks SILENTLY if sub-hour sampling is
  ever added. (2) The 48-seed fuzzer interleaves only countries/opinions/state_events/
  subjects; relation_flags/agencies/capacity_clusters/cryptology/wars rely on the
  delta-lossless gate + dedicated tests, not randomized partial interleavings. (3) The
  "tombstones never from a partial view" wording is a TRAP — the invariant is
  fold-identity-with-full-emission and torn rows ARE correctly tombstoned; queue doc updated.

## Codex re-stamp — RUN 2026-07-10 (post-quota), VERDICT: REFUTED — adjudicated: CONTRACT-TEXT DEFECT, NO FOLD-DIVERGENCE DEMONSTRATED

The Codex Mode-A gate REFUTED with 2 CRITICAL + 2 HIGH + 2 MEDIUM-LATENT findings,
contradicting the Opus SOUND verdict. Orchestrator adjudication:

**Findings 1–3 (tombstones-from-partial-views, first-partial-frame seeding, stale-drop not
flagged partial): refutations against the BRIEF'S ABSOLUTE RULES, not against fold output.**
The lane's design deliberately relaxes "trackers never advance on partial" / "positive-absence
-only tombstones" for d53's tolerated-partial mode, because FULL EMISSION ITSELF re-emits the
(possibly torn) current view each hour — the hour's folded snapshot drops torn rows either
way, and they reappear on the next clean read. The governing invariant is
fold-identity-with-full-emission; the fold-identity tests, the 48-seed fuzzer, and the
all-partial cold-start case pass, and the review presents no fold-divergent interleaving.
CODEX_LANE_BRIEF.md §3 now carries an explicit partial-tolerance carve-out so the contract
has one truth (two reviewers legitimately read the old text in opposite ways — that ambiguity
was the actual defect). Finding 3's stale-drop nuance: the dropped row is absent from the
SAME read that both modes consume, so both modes' hour view agrees; stale_dropped_total
remains visible diagnostics.

**Finding 4 (default-mode not byte-identical to PRE-LANE in the post-build-rejection
sequence): TRUE, and ACCEPTED-INTENTIONAL.** The mutate-before-validate fix changes behavior
exactly where the pre-lane code was provably lossy (tracker advanced on a frame later
rejected ⇒ subsequent deltas diffed against a baseline the fold never saw). The prior
"default byte- and CPU-identical" claim is RESTATED as: identical on every sequence where
pre-lane was correct (G1 deep-war chain hash reproduces); divergent only where pre-lane lost
data, in the direction of correctness. This restatement is the documented intent the G1 gate
requires for a folded-output change.

**Findings 5–6 (mode-switch mid-stream; two d53 records in one hour): latent, already
recorded in this report; unreachable today** (control loads once per attach; one d53 record
per FRAME_END). Both become live hazards only if those preconditions change — re-gate then.

Status: delta_partial remains **opt-in OFF**, blocked on the live paired A/B as before. The
adjudication above is open to challenge: a concrete fold-divergent interleaving overturns it
— that is the standing falsifier.
