# focus_research_decisions_depth_live

Lane-local production stack for focus/research/decision depth records beyond the
already-proven base compact `focus_live`, `tech_live`, and `decisions_live`
domains.

This crate does not edit shared collector or recorder entrypoints. It consumes
the production compact H4CR stream from the DLL/Rust recorder, derives depth
transition rows in Rust, emits a lane-local compact stream, and gates that
stream with fail-closed decoder checks.

## Scope

- Focus transition markers from compact `focus_live` deltas:
  current/continuous start, stop, switch, completed-count increase, inferred
  completion token when the previous current focus is extractable, and progress
  regression/anomaly markers.
- Research slot speed/ETA rows from compact `tech_live` deltas:
  same-tag/same-slot/same-tech live progress deltas, observed raw speed per game
  hour, base required points, ETA hours, and rollover/regression markers.
- Decision transition rows from compact `decisions_live` deltas:
  active timed-decision start/stop/day-delta/state-change rows plus per-country
  status-vector counts for active timed, targeted, targeted timed, re-enable,
  alternate re-enable, and live timed rows.
- Fail-closed badge rows for fields that are intentionally omitted until a
  production-safe root exists:
  focus pause, targeted/cooldown decision target fields, and direct engine event
  hooks.

## Production Domain

The integrated production compact payload uses domain id `45` and domain name
`focus_research_decisions_depth_live`. Domain `44` is reserved for
`supply_logistics_attrition_live` and must not be reused by this lane.

## Proof

From the repo root:

```powershell
cargo test --manifest-path replay_native\lanes\focus_research_decisions_depth_live\Cargo.toml

cargo run --manifest-path replay_native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin proof_from_stream -- `
  --input replay_native\runs\compact_country_870ebf_speed5_20260701_v2\collector_stream.bin `
  --out-dir replay_native\runs\focus_research_decisions_depth_live_20260701_v2_badges

cargo run --manifest-path replay_native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin gate_depth_stream -- `
  --input replay_native\runs\focus_research_decisions_depth_live_integrated_870ebf_20260701_v1\collector_stream.bin `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\integrated_depth_gate_20260701_v1
```

The proof command writes:

- `depth_stream.bin`: derived H4CR compact stream containing one lane-local data
  record per source frame.
- `proof_manifest.json`: source frame/domain counts, skipped-hour gate, derived
  row counters including `badge_rows` and `badge_rows_by_kind`, and
  parse/gate errors.
- `sample_events.jsonl`: first derived rows for manual inspection.

The direct depth gate over the integrated domain `45` stream writes
`depth_gate_manifest.json`. The current integrated proof has `89` depth data
records, `9` focus event rows, `0` exact completed-focus closure rows, `1`
inferred completed-focus closure row, `267` badge rows, and zero parse errors.

Current-build root validation for fail-closed depth fields:

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir replay_native\runs\focus_research_decisions_depth_live_current_roots_20260701_v3

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v4 `
  --sample-limit-per-list 24

python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_body_candidates.py `
  --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v4 `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_body_candidate_gate_20260701_v1

python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_residual_blockers.py `
  --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v4 `
  --depth-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\integrated_depth_gate_20260701_v1\depth_gate_manifest.json `
  --integrated-manifest replay_native\runs\focus_research_decisions_depth_live_integrated_870ebf_20260701_v1\native_session_manifest.json `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\residual_blocker_gate_20260701_v1
```

See `CURRENT_BUILD_ROOTS.md` for the validated/rejected roots and
`INTEGRATION_PATCH_PLAN.md` for the direct DLL/Rust patch plan.

Current residual status is `NOT COMPLETE`: focus pause, direct completed-focus
token closure in compact production, targeted/cooldown decision body fields, and
direct engine event hooks remain blocked or badged.

## Integration Plan

The exact shared patch plan is in `INTEGRATION_PATCH_PLAN.md`. Keep this lane
isolated until the orchestrator grants a shared integration slot.
