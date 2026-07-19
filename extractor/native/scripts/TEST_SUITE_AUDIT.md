# Rust test-suite audit

Date: 2026-07-19

Scope: `extractor/native`, `replay_interface/native`, and `portable_replay/native`. This audit is structural and read-only with respect to test execution; it did not start a Cargo build or test battery.

## Inventory

- 4,835 Rust test annotations: 2,799 extractor, 1,920 replay, and 116 portable replay.
- 21 ignored annotations.
- The structural parser resolved 4,827 concrete test functions; macro/attribute shapes outside the parser remain counted in the annotation inventory.
- 10 exact normalized-body duplicate groups.
- 0 obvious tautologies such as `assert!(true)` or a simple value compared with itself.
- 60 non-ignored external-resource early-return candidates.
- 77 broad slow-path candidates. This is a review list, not a measured-slow count; many test deadline behavior and are fast.

Run `python extractor/native/scripts/Audit-RustTests.py --json` for the complete current candidate inventory.

## Changes made from the audit

1. `replay_app` test binaries `soak` and `soak_f2` are now the explicit `Soak` tier. The measured outlier `f2_correctness_budget_and_trend_gates` took 111.93 seconds in the observed full run and no longer enters ordinary or full-offline validation.
2. Game-install and capture-corpus tests are now selected by the `Evidence` profile and excluded by the ordinary and CI default filters. This includes game-UI integration binaries that resolve `BATTLE_PLAN_GAME_ROOT`/`BATTLE_PLAN_GAME_DIR` and named unit tests that previously returned success when those resources were absent.
3. Routine `Fast`, `Integration`, and `Test` runs fail fast. `Final`, `Soak`, `Evidence`, and `Release` retain complete failure reporting.
4. A 10-second slow marker now identifies new ordinary-tier outliers without terminating them. Soak and evidence profiles have tier-appropriate slow markers.

## Duplicate-test disposition

No test was deleted solely because its body matched another test.

- Five duplicate groups exercise the independently owned domain-44 and domain-52 gates. They remain distinct until the production implementation is actually shared.
- D3D state and stream-output duplicates protect separate map-viewer and replay-application consumers.
- Tooltip codec duplicates protect separate payload families.
- The alert duplicate crosses module-unit and public integration boundaries.
- SHA-256 standard-vector duplicates belong to independent proof binaries. Consolidating their implementation may be reasonable later, but deleting one test before that refactor would remove coverage of the copied implementation.

The exact duplicates therefore have distinct failure boundaries. Their runtime is negligible compared with compilation and soak work, so deletion would provide no meaningful speedup.

## Test legitimacy rule

A default-tier test must protect a distinct production contract or a named regression, be deterministic, and fail when that behavior is broken. It must not require an installed game, captures, RenderDoc, hardware, a live process, long sleeps, stress loops, or soak budgets.

Move rather than delete when a test is valuable but belongs to soak, performance, evidence, or live validation. Delete or consolidate only after proving all of the following:

- another retained test fails for the same production defect;
- the candidate adds no separate public, producer/consumer, domain, portability, or fail-closed boundary;
- its expected value is not the only independent oracle;
- it is not the sole regression test for a previously observed defect.

Tests that silently return when external evidence is missing are not ordinary passes. They belong to the explicit `Evidence` tier, whose prerequisites must be satisfied before its result is used as promotion evidence.
