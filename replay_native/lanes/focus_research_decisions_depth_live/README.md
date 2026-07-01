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

The exact-completed production slice proof is gated separately:

```powershell
cargo run --manifest-path replay_native\lanes\focus_research_decisions_depth_live\Cargo.toml --bin gate_depth_stream -- `
  --input replay_native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\collector_stream.bin `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\exact_completed_gate_20260701_v2

replay_native\target\release\pack_replay.exe verify `
  replay_native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\native_session.battleplan
```

That proof has `144` depth records, `1` exact completed-focus row, `0`
inferred rows, `0` unresolved rows, `432` badge rows, zero parse errors, zero
skipped hours, and verified `.battleplan` output. The shared collector hunk
needed for that behavior is not applied by this lane; it is documented for the
orchestrator in `SHARED_RESIDUAL_PATCH_PLAN.md`.

Current-build root validation for fail-closed depth fields:

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir replay_native\runs\focus_research_decisions_depth_live_current_roots_20260701_v3

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v5 `
  --sample-limit-per-list 32

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --sample-limit-per-list 10000

python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_body_candidates.py `
  --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_body_candidate_gate_20260701_v3_full_decisions

python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_variant_candidates.py `
  --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --with-static-defs `
  --out-dir replay_native\runs\focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_decision_target_save_oracle.py `
  --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_target_save_oracle_gate_20260701_v1

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_decision_numeric_target_save_oracle.py `
  --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_numeric_target_save_oracle_gate_20260701_v1

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_candidates.py `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_wide_candidates.py `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2 `
  --start-offset 0x0 `
  --end-offset 0x80

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_static_gap_candidates.py `
  --wide-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2\focus_pause_wide_candidate_gate.json `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_static_gap_gate_20260701_v2

python replay_native\lanes\focus_research_decisions_depth_live\tools\watch_focus_pause_fixture.py `
  --focus-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2\focus_pause_candidate_gate.json `
  --duration-s 12 `
  --sample-ms 500 `
  --min-game-hours 2 `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_fixture_watch_20260701_v1

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_candidates.py `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1

python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_static_contracts.py `
  --event-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1\event_hook_candidate_gate.json `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_static_contract_gate_20260701_v1

python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_residual_blockers.py `
  --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions `
  --depth-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\integrated_depth_gate_20260701_v1\depth_gate_manifest.json `
  --integrated-manifest replay_native\runs\focus_research_decisions_depth_live_integrated_870ebf_20260701_v1\native_session_manifest.json `
  --focus-pause-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2\focus_pause_candidate_gate.json `
  --focus-pause-watch replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_fixture_watch_20260701_v1\focus_pause_fixture_watch.json `
  --focus-pause-wide-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2\focus_pause_wide_candidate_gate.json `
  --focus-pause-static-gap-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_static_gap_gate_20260701_v2\focus_pause_static_gap_gate.json `
  --event-hook-static-contract-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_static_contract_gate_20260701_v1\event_hook_static_contract_gate.json `
  --completed-depth-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\exact_completed_gate_20260701_v2\depth_gate_manifest.json `
  --completed-integrated-manifest replay_native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\native_session_manifest.json `
  --decision-body-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_body_candidate_gate_20260701_v3_full_decisions\decision_body_candidate_gate.json `
  --decision-variant-gate replay_native\runs\focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions\decision_variant_candidate_gate.json `
  --decision-target-save-oracle-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_target_save_oracle_gate_20260701_v1\decision_target_save_oracle_gate.json `
  --decision-numeric-target-save-oracle-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_numeric_target_save_oracle_gate_20260701_v1\decision_numeric_target_save_oracle_gate.json `
  --event-hook-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1\event_hook_candidate_gate.json `
  --decision-target-compact-schema-proved `
  --integrated-replay-verified `
  --completed-replay-verified `
  --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\residual_blocker_gate_20260701_v10
```

See `CURRENT_BUILD_ROOTS.md` for the validated/rejected roots,
`INTEGRATION_PATCH_PLAN.md` for the domain-45 integration shape, and
`SHARED_RESIDUAL_PATCH_PLAN.md` for residual-specific shared hunks that require
an orchestrator slot.

Current residual status is `NOT COMPLETE`: direct completed-focus token closure
is a validated slice when compact focus source rows carry completed vectors.
Focus pause has the full `CFocusStatus+0x00..+0x7F` bounded object range
rejected as the paused-boolean home. The original `+0x20..+0x37` window rejects
`24` byte candidates; the wider live v2 scan rejects the other `100`
outside-window candidates except for the `+0x3C..+0x3F` gap; and the static v2
gap gate rejects those final `4` bytes by CFocusStatus vtable-method qword
access evidence at `+0x38`. The bounded watcher still records the required
paused=true oracle contract for any future root outside `CFocusStatus`.
Decision body extraction is partially unblocked: targeted-timed country-target
and numeric-target rows now have validated target fields, while cooldown/day/
state details, numeric state/province semantic classification, unmatched
transition rows, and direct engine event hooks remain blocked or badged.

The decision target save-oracle gate validates a real target field for part of
the targeted-timed body work: for vtable `0x27D5038`, `record+0x20` is the
country target `tag_db_id` for the exact save/live matched country-target
subset. The proof compared the full `3384` live targeted-timed rows against the
current autosave targeted-decision rows, validating `1061` live country-target
rows across `180` decision tokens. It leaves `33` live country-target transition
rows plus cooldown/day/state body fields blocked. The numeric target
save-oracle gate validates `record+0x24` as the numeric target id for `2285`
exact save/live matched `target=<number>` rows; it leaves `4` live-only and `3`
save-only numeric transition rows plus semantic classification of the numeric
ids as state/province/body ids blocked. The full v6/v3 variant refresh sampled `3384` targeted-timed records
across `262` decision tokens and rejected `49` definition-local country-tag
slices.

The lane-local compact schema/decoder/gate now has a production-shaped target
row for those validated subsets. The depth payload uses the reserved header
prefix for `decision_target_count` and emits fixed target rows
`owner_tag_db_id, target_id, flags, source_kind, decision`. Rust tests prove
derivation from an extended compact `decisions_live` fixture, acceptance of the
country/`record+0x20` and numeric/`record+0x24` flags, and rejection of invalid
target metadata.
This is still not shared-production shipped until the orchestrator applies the
collector/recorder integration slot.

The event-hook candidate gate rejects the named decision-event string route and
the named focus-completion callback string route for this build. The follow-up
static-contract gate rejects all `5` remaining static text-xref anchors as
registration/catalog arguments, not runtime callbacks. Each anchor is inside
function `0x82CB0..0x11896F`, which has `10697` string references; the local
contract loads the anchor string into `r8`, loads a registration key into `edx`,
sets a global registry pointer in `rcx`, and calls `0x24AFC10`. No remaining
string/xref route has a runtime focus/research/decision object argument
contract, so direct hooks remain blocked.

## Integration Plan

The exact shared patch plan is in `INTEGRATION_PATCH_PLAN.md`. Keep this lane
isolated until the orchestrator grants a shared integration slot.
