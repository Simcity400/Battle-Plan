# Wave 12 board (launched 2026-07-12, base 6fd03c653)

10 Codex Mode-C lanes, strict file-ownership partitioning — NOTHING in this wave touches
collector/src/lib.rs (single-owner file; the census/stage-A batch owns the main tree
until its round-5 gate SHIPs, then lib.rs C0 fixes get one serialized owner applying the
w12-libr-pairs-dossier patches). Worktrees + branches: ~/hoi4_lanes/w12-*; tids:
~/hoi4_lanes/threads/w12-*.tid; prompts: ~/hoi4_lanes/prompts/w12-*.txt.

| Lane | Type | Fence | Deliverable |
|---|---|---|---|
| w12-d53-badges | attribution+fix | lanes/diplomacy_intel_operatives_live | W12_D53_BADGES.md dossier + in-fence fixes (bits 16/17/20/22) |
| w12-d51-badges | attribution+fix | lanes/scripted_state_flags_variables_live | W12_D51_BADGES.md (bits 11/12/13) |
| w12-d48-fold-skip | attribution+fix | lanes/construction_* | W12_D48_FOLD_SKIP.md (fold problems; world_fold diffs proposed only) |
| w12-libr-pairs-dossier | analysis ONLY | documents/dossiers | W12_LIBRS_PAIRS.md — d45/d49/d50/d54/d60/d65/d43/d47 pairs + proposed lib.rs diffs |
| w12-pass2-spans | tooling | scripts/c0_pass2_spans.py | census pass 2: per-hour spans + SO-ledger reconciliation + W12_PASS2_RESULTS.md |
| w12-design-autoattach | design | AUTO_ATTACH_DESIGN.md | dual-order attach (task #36) |
| w12-design-campaign-id | design | CAMPAIGN_IDENTITY_DESIGN.md | engine-derived campaign identity (task #34; d68/d57 first) |
| w12-design-stitching | design | STITCHING_DESIGN.md | rollback segments + later-supersedes (task #35) |
| w12-design-c1-stageb | design | C1_STAGEB_REGISTRY_DESIGN.md | declaration registry + epoch tokens per G4 amendments |
| w12-test-debt | tests | recorder_host tests only | politics_window flake, RM file-symlink variant, temp-dir collision audit |

MERGE PROTOCOL (per documents/ merge-review canon, FULL REGIME): every lane gets an
independent adversarial review gate before merge; citation_check on dossiers; design docs
get a design-review gate; NO auto-union merges. Lanes were told: out-of-fence changes =
proposed diffs in the dossier, never applied.

IN PARALLEL (main tree, not lanes): census v3.2 + stage-A residue under combined-round5
gate (expected terminal) -> commit on SHIP closes C1 stage A (task #30).

USER-GATED (not in wave): live captures (d69 <9-save confirm, d47/d69 deep-war soak,
cross-modset matrix #12), P5 speed-5 acceptance.

## Wave 12b (launched 2026-07-12 evening, USER call "launch them all now" — capacity law
overridden knowingly; wedged lanes bail clean + resume by tid). Source:
REPLAY_REQS_RECONCILIATION_2026-07-12.md (the replay-requirements fill-in).

| Lane | Type | Fence | Deliverable |
|---|---|---|---|
| w12b-fold-wave1 | fold code (world_fold.rs OWNER) | world_fold.rs + tests | 8b pair/faction/capitals complete, 7b ruling_party, naval presence + combat-bubble folds from existing payloads, D56 unit statement, 9b delta audit |
| w12b-fow-scope | dossier | dossiers/ | A14 FoW CPU-side intel-state scoping (item 13 REQUIRED) |
| w12b-events-decisions-scope | dossier | dossiers/ | A18 fired events + A21 decision state + 8b war aggregates scoping |
| w12b-locjoin-verify | verify+script | scripts/locjoin_coverage.py + dossier | item 9: does the loc-join artifact ship in production + cover key families |
| w12b-speedpause-scope | dossier | dossiers/ | item 4 speed-change events reader design |
| w12b-dlc-meta | code | modset.rs + main.rs validator/tests | item 4 DLC display names + localisation language (closed-grammar validator extended in-step) |

Also relaunched in the same volley: combined-round6 gate rerun (round6b), d51-badges
resume 2, test-debt resume. c1-stageb resume still in flight from earlier.
LIB.RS QUEUE (unchanged): counter-badge fields, world_tension, speed reader — after the
census batch commits and w12-libr-pairs-dossier's patches are adjudicated.

## Status ledger 2026-07-12 late (credit wall #4 at ~21:30, reset 00:50 Jul 13)

COMPLETE, lane-committed in-worktree (orchestrator commit where the sandbox could not
take index.lock), REVIEW GATE STILL OWED before merge:
- w12-design-autoattach  -> AUTO_ATTACH_DESIGN.md          (b42bdb420)
- w12-design-campaign-id -> CAMPAIGN_IDENTITY_DESIGN.md    (4637b6dce) — d68 source
  corroborated live: session_header_live flags_or=0 on d04stageA_211128
- w12-d53-badges         -> W12_D53_BADGES.md              (f40229d7e) — all 4 badges
  chronic capture gaps 717/717; no safe in-fence clear; proposed diffs in dossier
- w12b-events-decisions-scope -> W12B_EVENTS_DECISIONS_SCOPE.md (e07fef384)

IN FLIGHT (sessions alive from before the wall; retry internally): w12-d48-fold-skip,
w12-pass2-spans, w12-design-stitching (doc written, uncommitted), w12-libr-pairs-dossier,
w12b-fold-wave1, w12-design-c1-stageb (doc written, uncommitted), w12b-fow-scope,
w12b-dlc-meta, w12-d51-badges, w12-test-debt.

QUEUED post-reset (relaunch_post_reset_20260713.sh): combined-round6e gate (CRITICAL —
unblocks census+stage-A commit), w12b-locjoin-verify fresh (40-try refire loop exhausted
into the wall), w12b-speedpause-scope fresh (bailed on wedge; provisional intel in
dispatch queue).
