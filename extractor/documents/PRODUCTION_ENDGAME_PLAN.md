# PRODUCTION ENDGAME PLAN — plan of record (2026-07-11)

USER DIRECTIVE: full plan of attack, ready to execute the moment Codex is back. Maximum
parallelism. End state = PRODUCTION: capture times optimized, replay sizes optimized,
nothing missed, everything VERIFIED (retroactive-audit bar: census/coverage/data-closure,
adversarially gated, live-validated — "finished" means finished + verified, per the standing
scope ruling and audit mandate).

Standing laws bind throughout: zero silent data loss (zero-silent-gap accounting + bounded
skew), never mask failure bits, weaker-PC CPU budget, measure-first (no lever ships on a
model), one Codex fix round per review then orchestrator fixes, resumability law on every
Codex call, ≤15-min live turnarounds, `replay_interface/` untouched.

---

## 0. Where we stand (evidence, not vibes)

DONE + LIVE-VERIFIED (Codex re-stamp still owed where noted):
- **Bytes: COMPLETE.** 163–173x production compression. Window-27 LDM shipped; container
  levels exhausted; domain grouping = artifact; d53 delta CPU-negative; columnar measured
  NULL (−2.9%, `scripts/d53_columnar_probe.py`). No lossless layout lever remains.
- **d47 RESTORED** (831,039 rows/509h vs all-time 0; parse errors 0; dead_roots 1) via
  root-refresh machinery + duplicate-char dedup. GATE OWED.
- **d69 RESTORED** (status 0, full fold coverage, first ever) via stale-slot + vacant/
  tombstone member classification. GATE OWED.
- **d43** cold/empty semantics hardened (2 gate rounds, SOUND); sweep-completeness law
  (`coverage_complete()`) protects all adopting consumers; sweep enumeration holes fixed.
- **Modset binding (O5/M2)** live-validated (`resolved=true`, 20/24 DLCs). GATE OWED.
- **O4/U1** RTTI re-derivation self-proven 51/51. **O7** war-segment accounting bit shipped.
- **Perf shipped:** SEH read+region cache (3.57x), W0 (−43% d53 warm), prefetch relation=8,
  reader_concurrency=12, deferred fold, zstd-at-boundary, hour-latch (default OFF, gated).
- **Serial-wall lever IMPLEMENTED** (1cac48222): 10 post-join readers moved into the worker
  scope (the read phase was 46.6ms concurrent THEN 43.7ms serial = 92.3ms span).
  **A/B RUNNING NOW** (swab_A*/swab_B* windows; analyzer staged). GATE + VALIDATION OWED.

OPEN LEVERS / DEBTS (this plan closes every one):
perf: serial-wall validation, candidates prefetch, hour-latch default decision, pace
exogeneity, speed-5 re-acceptance. correctness: O8 SourceDisposition (green-gates enabler),
d43 elapsed-0 warmup, d47/d69 deep-war soak. robustness: M1 modset-resolved catalogs,
vanilla-tuned validation bounds audit, cross-modset matrix, U-series post-patch oracles
(beyond vtables). production: retroactive audit reconciliation, dress rehearsal, runbook.

---

## 1. WAVE G — the gate storm (hour 0 of Codex's return; ALL PARALLEL, Mode A read-only)

Every lane below is independent; dispatch all five in the first hour. Resumability law on
each; thread ids to `~/.claude/codex_threads.log`; usage-limit deaths queue + resume.

| # | Gate | Scope | Verdict unblocks |
|---|------|-------|------------------|
| G1 | **Final net gate** (queued, prompt amended in CODEX_DISPATCH_QUEUE.md; resume tid recorded) | 8e10e70ff + 28f2043e1 + d5bb02247 + ebdc5bc4b + 1fbd9f3a4 + 2a9dc5091: sweep completeness, d69 stale/vacant/tombstone, d47 char-dedup key + degrade rule, revert completeness, O7 bit | d47+d69 verified-closed; master advance |
| G2 | **Modset gate** (queued) | e9f21a1c8: fail-closed resolution, ambiguity, exe-hash race, census key format, FFI | M-series proceeds on a verified base |
| G3 | **Serial-wall gate** | 1cac48222: no-alias argument, gate-token starvation (12-cap vs +10 workers), join ordering, panic semantics, lane_wall telemetry meaning shift | Perf endgame ships on a verified base |
| G4 | **O8 design review** (design already in CONFIRMED_COMPLETE_DESIGN.md §2–5) | SourceDisposition types, ExpectedAbsent declaration rules, prerequisite plan, forbidden list; adversary attacks the tolerance semantics | Wave C implementation starts |
| G5 | **Audit-completeness gate** | AUDIT_LEDGER.md vs merged-work list since ff1d37bed: name every merged change lacking its audit row | Wave F reconciliation list |

Fix rounds: one per gate (Codex), then orchestrator fixes directly. Lossless-critical
verdicts get the standing extra final gate.

## 2. WAVE P — perf endgame (live windows serialize on the ONE game; analysis parallel)

Order chosen so each window answers one question; ~15 min each; gate-bracket every run
(pause/unpause = discard). DLL-hold gotcha: -SkipBuild between same-binary windows, 35s+
spacing, NEVER cargo-build while a capture is in flight.

- P1 **Serial-wall A/B verdict** (already capturing): expect main_wall 43.7→<10ms, span
  92→~55-65ms. If confirmed → G3 dispatch; if null/regression → revert 1cac48222, record.
- P2 **Candidates palindrome, clean re-run** (driver ready: ab_candidates_driver.ps1) on
  the post-P1 binary: [32,16,0,0,16,32]. Decide `PREFETCH_DIST_CANDIDATES` default like
  relation-8. (~28 min)
- P3 **Hour-latch decision run**: one 600s window `--hour-latch` on the P1 binary — latch
  invariant + frame-cost check (ABBA already green; this is the post-serial-wall re-check).
  Then USER CALL on default-ON. 6-core portability run stays queued for the other rig.
- P4 **Pace-exogeneity experiment** (decides whether frame-time cuts speed the GAME up):
  fixed save+speed windows with domain subsets ~40/~80/~151ms; compare hour-interval
  distributions. Uses the subset presets; driver to write. (~45 min, can run overnight)
- P5 **Speed-5 re-acceptance** (the benchmark of record, after P1–P3 land): full production
  preset, deep-war save, 600s: report frame p50/p95, skipped hours, latch accounting,
  ms/game-hour. Acceptance: p95 < 100ms at speed 5 AND zero-silent-gap invariant holds.
  This number goes in the README as the production spec.

Perf exit statement after P5: every known lever is shipped, measured-null, or explicitly
declined with evidence (the known-NULLs list). Frame time is then CRITICAL-PATH-bound on
d53's ~46ms concurrent wall — further cuts need d53-internal work (W1.1 outcome-split
timing exists as the probe; only pursue if P5 misses acceptance).

## 3. WAVE C — correctness endgame (implementation lanes; parallel with Wave P analysis)

- C1 **O8 SourceDisposition** (XL; starts after G4; Mode C worktree lane if the sandbox
  works, else orchestrator implements with Codex gating each stage):
  stage 1 compact_contract types + wire format (versioned; never touches failure bits);
  stage 2 collector per-segment dispositions (d43 known-empty, d17 war segment, bridge
  domains, sweep-cold);
  stage 3 recorder gate: hourly requirement consumes dispositions — SOURCE from a declared
  ExpectedAbsent segment no longer fails the run; everything else unchanged;
  stage 4 fold/decoder passthrough + tests + G1-chain fold-identity proof.
  EXIT: a clean vanilla production run ends GATES GREEN (today's chronic
  compact_source_unavailable/partial problems resolve to typed dispositions). This is the
  single change that makes "production run succeeded" mean something.
- C2 **d43 elapsed-0 warmup preflight** (S): needle warm-up before the first frame with
  bounded timeout (fail closed) — kills the last cosmetic SOURCE paint. After C1 lands it
  may be unnecessary (disposition covers it) — decide then, don't double-build.
- C3 **d47/d69 deep-war soak** (live, 1h+ window piggybacked on P4/P5 captures): d47 rows
  every hour, dead_roots ≈ churn, parse errors 0; d69 status 0 throughout; refresh/hold
  telemetry sane. Also re-check PARTIAL-upsert pricing from the same manifest (stays
  deferred unless churn-loss rate materially exceeds ~1 frame/session).

## 4. WAVE R — robustness endgame (fully parallel with C; different files)

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
- R3 **Cross-modset matrix** (USER-DEPENDENT — the ask): one session each on (a) a small
  cosmetic mod, (b) a DLC-disabled combo, (c) a major overhaul (Kaiserreich-class).
  Per session: modset resolves in manifest; per-domain rows/status vs vanilla baseline;
  R2's bounds report zero unexplained trips; d53 offsets-vs-scan shadow referee green.
  EXIT: the mod-flexibility promise is evidence, not architecture.
- R4 **U-series remainder** (S, after O4): patch-day runbook — rtti_rederive --rederive,
  d53 shadow-mode referee capture, sentinel checks, bounds re-validation — written as a
  single checklist doc with commands. (The vtable half is already self-proven.)

## 5. WAVE F — production finalization (after G1–G5, P5, C1, C3; R3 may lag)

- F1 **Retroactive audit reconciliation**: close G5's list — every merged change has its
  AUDIT_LEDGER row (gate verdict, live validation, citation check). The audit bar is the
  standing definition of verified; no exceptions ride into the release.
- F2 **Dress rehearsal** (the production acceptance run): boot → full multi-hour session at
  speed 5 (deep war) → gates GREEN (post-C1) → deferred fold → .battleplan → replay_decoder
  spot-checks + REPLAY_DATA_REQUIREMENTS.md coverage read-through (every required item
  present in the artifact; update the doc in the same commit if any drift). Byte size and
  frame numbers go in the manifest of record.
- F3 **Production runbook + release**: one doc — launcher invocation (preset, latch flag
  per P3 decision, modset expectations), the acceptance numbers from P5/F2, patch-day
  checklist (R4), known-NULLs list (so nobody re-litigates dead levers), owed-maintenance
  list. Tag the release commit; update EXTRACTION_OPTIMIZATION_PLAN.md to CLOSED status.
- F4 **Handoff note to the replay layer** (via EXTRACTION_INTERFACE_REQUESTS.md): stream
  spec version, disposition semantics (C1), modset manifest fields (M2), anything the
  replay engine must consume differently.

## 6. Parallelism map (what runs simultaneously)

```
Codex returns ──┬─ G1 ─┬────────────────→ master advance
                ├─ G2 ─┤
                ├─ G3 ←─ P1 verdict      (gates: 5-way parallel, Mode A)
                ├─ G4 ──→ C1 (staged impl; Codex gates each stage)
                └─ G5 ──→ F1
game (serialized): P1 → P2 → P3 → P4 → P5 → C3 soak → F2 rehearsal
orchestrator (between windows): C2, R2 wiring, R4 doc, F3 drafting
Codex (after gates): R1 lane, R2 survey, C1 stage gates
user: P3 latch default call · R3 modded sessions · credits
```
Live windows are the only hard serial resource. Codex lanes and orchestrator work fill
every gap. If the Mode-C sandbox is still broken (CreateProcessWithLogonW:2), Codex runs
gates/surveys/design only and the orchestrator implements — same plan, same order.

## 7. Definition of DONE (production)

1. Every commit since the last verified baseline carries a SOUND adversarial verdict
   (G1–G3, C1 stages, R1) — no unreviewed lossless-critical code in the release.
2. P5 acceptance: speed-5 deep war, p95 frame < 100ms, zero silent gaps (accounting
   invariant), documented ms/game-hour.
3. Bytes: ~163–173x documented with the known-NULLs table (nothing left un-tried or
   un-buried).
4. A clean vanilla production run ends GATES GREEN (C1) and the dress rehearsal artifact
   (F2) satisfies REPLAY_DATA_REQUIREMENTS.md end-to-end.
5. Every extraction domain: rows flowing or a TYPED disposition — zero silent absences
   (d47, d69, d17 all closed; the disposition system covers the rest).
6. Mod/update robustness is EVIDENCED: R3 matrix run (or explicitly waiting on user
   sessions, listed as the only open item), patch-day checklist self-proven where possible.
7. AUDIT_LEDGER reconciled; runbook + release tag exist.

## 8. Immediate actions on "Codex is back" (copy-paste order)

1. Health-probe with a REAL xhigh job (cheap-call success is not quota proof).
2. Dispatch G1–G5 in five parallel Mode-A calls (prompts: G1/G2 in
   CODEX_DISPATCH_QUEUE.md; G3/G4/G5 from this doc's scope lines).
3. Slot P1's verdict into G3's prompt if the A/B has landed by then.
4. Start P2 on the game; draft C1 stage-1 types while gates run.
5. As verdicts land: fix rounds (one each) → master advance → start C1/R1 lanes.
