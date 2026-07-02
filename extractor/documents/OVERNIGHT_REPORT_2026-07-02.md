# Overnight run report — 2026-07-02

## TL;DR
All accepted work is **committed and green** at `8dd85e295` (Rust workspace 73 test-result blocks, Python 1085 passed, `replay_interface/` untouched). The overnight queue did **not** fully finish: the Codex sandbox began failing intermittently (`windows sandbox: helper_unknown_error: apply deny-read ACLs`) partway through, which blocked the remaining delegated lane integrations. I stopped rather than force-run through an unstable environment and risk a broken tree. One genuinely serious correctness issue was found and contained (domain 53). Nothing needs the game except the final live proof.

## What landed (committed at 8dd85e295)
- **Lanes 55, 56, 48, 47** integrated (collector + recorder decode + world_fold), validated slices only, fixture-blocked scope fail-closed.
- **Fold backlog fully closed** — every decoded domain now reaches snapshots (occupation 52, map_detail 23, supply 44, market 43, OOB/naval/air cluster 27–42/46, projects 41, depth 45, scripted_state 51, div_equipment 26, weather 24). Established field-ownership merge (no cross-domain clobber), decoder-strict folds, three-state semantics.
- **Fail-closed hardening** — shared `compact_status_is_failure()`, first-hour delta-baseline guard, failed-run container quarantine (`.failed.battleplan`, prior container preserved), unfolded-domain annotation. Fixed a **latent tombstone-loss bug in 9 already-accepted domains** (empty-vs-source-unavailable conflation).
- **Production profile** (`--production`, derived from descriptor status, MAX_SCHEDULED_DOMAINS guard) + drift-guarded `domain_registry.json`.
- **Static-artifact packing** (domain-59 catalog + R3 localisation) and **localisation joins** (ideas/techs/decisions + coverage report).
- **Domain 57 roots** live-validated by passive probe.

## The serious find: domain 53 — WITHDRAWN, contained
Domain 53's extractor stamped `SOURCE_PROOF_..._870EBF` on offsets whose probe evidence was for the **older 16022851 build** (country vtable, subject/war/agency RVAs all differ). That is the exact fail-closed violation this project guards against — claiming validation it did not have.

Action taken: descriptor downgraded to `ActiveIntegration`/not-schedulable (**out of `--production`** — verified via the `id_for_scheduled_name(...) == None` test), collector fail-closes the whole domain, and the subject/cryptology/operative decoders are conservatively disabled. The full-extraction lane test is `#[ignore]`'d with a re-validation pointer. **Domain 53 cannot reach production.** It needs live-870ebf root re-validation before re-enabling. (The Codex sandbox died mid-fix, leaving the tree broken; I hand-repaired it to green rather than lose the session — the hand-edits are small, behavior-preserving, and documented in-code.)

## Offset-provenance audit (task 19) — the key result for the live proof
Prompted by the 53 finding, I audited whether each lane's deep offsets are validated on the **current 870ebf** build:
- **56 naval, 48 construction, 54 battleplans**: explicit 870ebf evidence exists (probes / `save_geometry_870ebf_20260701_v1`). ✅
- **53 diplomacy**: withdrawn (above). ⛔
- **47 characters**: provenance **ambiguous** — has current-build revalidation machinery (`proof_live_roots.py`) but **no explicit 870ebf SHA stamp** on its included-row offsets, and some RTTI names are historical. ⚠️

**Decisive conclusion:** the fail-closed architecture (vtable-identity checks + skip-counting → `PARTIAL_DATA` + retain-on-unknown fold) means **stale offsets can never fabricate data — only produce source-unavailable/partial**. So the **live speed-five proof is the definitive offset validator**: any lane with stale 870ebf offsets will surface domain partial/source failures and fail the `--production` gate (which requires zero such failures). No emergency downgrades beyond 53. **Watch domain 47 specifically during the live proof** — if it shows partial/source-unavailable, its offsets need re-probing.

## Not done (blocked on the intermittent Codex sandbox)
These are ready (packets pre-authored, offline-drafted) but need reliable delegation to apply into shared files:
- **Lane 54** battleplans — packet + draft ready; 3 apply attempts all hit the sandbox ACL error.
- **Domain 43/45 slices** — `SLICE_PACKET.md` authored in each lane.
- **Domain 57 integration** — `INTEGRATION_PACKET.md` + `fold_r2_projection.rs` authored (offsets drafted for `build_profile`).
- **Cap-constant cleanup (#18)** — mechanical; deferred to avoid unreviewed shared-file edits overnight.

## Waiting on you
1. **Batched live speed-five proof** (needs the game unpaused). Run with `--production` + explicit `--scheduled-domain country_identity` + both static-artifact flags. Watch domain 47 for partial/source failures (offset-provenance check). Start with the strict domain-57 probe while still paused.
2. **Then** the bucket-2 discovery check-in you asked for.

Suggested first move on waking: re-verify the Codex sandbox is stable (a trivial write health-check), then resume the queue at lane 54.
