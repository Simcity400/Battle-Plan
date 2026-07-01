# Residual Acceptance Report

Lane: `focus_research_decisions_depth_live`
Residual assignment: `focus_research_decisions_full_residuals`
Domain id: `45`

## Completion

- Completion status: `NOT COMPLETE`.
- Whole residual slice complete: no.
- Backlog reduced: yes.
- Exact backlog removed or narrowed:
  - direct completed-focus token closure is a validated production slice when
    compact focus rows carry completed-token vectors;
  - all `24` byte candidates in `CFocusStatus+0x20..+0x37` are rejected as the
    focus paused-state boolean home;
  - the wider `CFocusStatus+0x00..+0x7F` v2 scan rejects `100` additional
    outside-window paused-state byte candidates;
  - the static v2 gap gate rejects the remaining `4` bytes at
    `CFocusStatus+0x3C..+0x3F` by vtable-method qword-access evidence;
  - the bounded focus pause watcher contract is built and proved against `2`
    unique live game hours;
  - `22` decision target/cooldown body candidates are rejected by the v2 body
    gate;
  - `49` definition-local targeted-timed country-tag slices are rejected by the
    v3 full-decision variant gate;
  - `active_targeted_timed_decision` vtable `0x27D5038` `record+0x20` is
    validated as the country target `tag_db_id` for `1061` exact save/live
    matched country-target rows across `180` decision tokens;
  - `active_targeted_timed_decision` vtable `0x27D5038` `record+0x24` is
    validated as the numeric target id for `2285` exact save/live matched
    `target=<number>` rows;
  - the lane-local compact schema, Rust decoder, and gate now accept that
    validated country-target subset and numeric-target subset as explicit
    decision-target rows and reject invalid target metadata;
  - `2` direct event-hook string/xref candidate routes are rejected by the v1
    static hook gate;
  - `5` remaining direct event-hook static text-xref anchors are rejected by
    the v1 static-contract gate as registration/catalog arguments rather than
    runtime callbacks.

Remaining blockers:

- Focus paused state is blocked until a same-build paused=true oracle is supplied
  to the watcher contract or a root outside `CFocusStatus` is validated with
  exact static or dynamic proof.
- Decision target/cooldown/body details are partially unblocked: country-target
  `record+0x20` and numeric target `record+0x24` are validated for matched rows,
  while unmatched transition rows, numeric target semantic classification, and
  cooldown/day/state fields remain blocked.
- Direct engine event hooks are blocked until a concrete hook/callback function
  address, argument contract, and live invocation proof are validated.

## Production Flags

For the exact completed-focus production slice:

- DLL-only: yes.
- Compact stream: yes.
- Rust decoded: yes.
- `.battleplan` verified: yes.
- Speed-five/current-equivalent hourly-or-lower: yes.
- Zero skipped hours: yes.
- Partial/source/worker/bridge failures: no.

The v10 residual gate records these flags in
`proofs/residual_blocker_gate_20260701_v10/residual_blocker_gate.json` under
`production_flags.exact_completed_stream`.

## Roots, Offsets, Functions

Validated focus roots:

- `CCountry+0x1370 -> CFocusTree`.
- `CCountry+0x1380 -> CFocusStatus`.
- `CFocusStatus+0x10` current national focus pointer.
- `CFocusStatus+0x18` current continuous focus pointer.
- `CFocusStatus+0x38` progress raw.
- `CFocusStatus+0x40/+0x48/+0x4C` completed vector data/capacity/size.

Rejected focus pause candidate window:

- Direct byte-shape rejections: `0x20`, `0x21`, `0x22`, `0x23`, `0x24`,
  `0x25`, `0x28`, `0x30`, `0x31`, `0x32`, `0x33`, `0x34`, `0x35`.
- Structural qword-overlap rejections: `0x26`, `0x27`, `0x29`, `0x2A`,
  `0x2B`, `0x2C`, `0x2D`, `0x2E`, `0x2F`, `0x36`, `0x37`.
- Inconclusive byte offsets left in this window: none.

Rejected focus pause candidates outside that window:

- Wider scan: `CFocusStatus+0x00..+0x7F`, excluding `+0x20..+0x37`.
- Rejected offsets by live wide v2 gate: `100`.
- Rejection evidence: known-field overlap for object header/vtable,
  `current_focus_ptr`, `continuous_focus_ptr`, progress, and completed-vector
  header fields; direct non-boolean byte values; pointer-qword overlap; dword
  scalar overlap.
- Static gap gate: `CFocusStatus` vtable RVA `0x2723D30`, `28` vtable entries
  scanned, `18` executable methods scanned, `2` methods with qword-sized access
  at `+0x38`, `0` methods with direct `+0x3C..+0x3F` access, rejecting
  `0x3C`, `0x3D`, `0x3E`, and `0x3F`.
- Remaining bounded `CFocusStatus+0x00..+0x7F` inconclusive offsets: none.

Validated/rejected decision roots:

- `CCountry+0xFA0 -> CDecisionStatus`.
- `CDecisionStatus+0x08` owner back-pointer.
- `CDecisionStatus+0x40`, `+0x88`, and `+0x130`: timed body layout rejected.
- `CDecisionStatus+0xA0`: existing active timed body remains valid for known
  timed rows.
- `CDecisionStatus+0xE8`: active targeted timed vector; `record+0x10 ->
  CDecisionDef` validates on `3384/3384` sampled rows. `record+0x20` validates
  as country target `tag_db_id` for `1061` exact save/live matched country-
  target rows. `record+0x24` validates as numeric target id for `2285` exact
  save/live matched `target=<number>` rows. Numeric target semantic
  classification, cooldown/day/state fields, `33` unmatched live country-target
  transition rows, `4` unmatched live numeric-target rows, and `3` unmatched
  save numeric-target rows remain blocked.

No focus/research/decision engine event hook function was validated. The static
event-hook gate rejects the named decision-event string route and the named
focus-completion callback string route. The static-contract gate rejects all
`5` remaining text-xref anchors as registration/catalog arguments rather than
runtime callbacks:

- `on_tech_research_completed`: string RVA `0x2AA30C0`, xref `0x10C8E4`.
- `on_research_complete`: string RVA `0x2AA6F30`, xref `0x11117A`.
- `on_research_complete_limit`: string RVA `0x2ADE1F8`, xref `0x1074F3`.
- `complete_national_focus`: string RVA `0x2AA2318`, xref `0x99AA4`.
- `uncomplete_national_focus`: string RVA `0x2ADEA48`, xref `0x108015`.

All five are inside function `0x82CB0..0x11896F`, which has `10697` string
references. Each local xref contract loads the string into `r8`, loads a
registration key into `edx`, sets a global registry pointer in `rcx`, and calls
`0x24AFC10`. Remaining static anchors from the string/xref route: `0`.

## Proof Commands And Results

```powershell
python -m py_compile replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_residual_blockers.py replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_body_candidates.py replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_variant_candidates.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_candidates.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_wide_candidates.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_static_gap_candidates.py replay_native\lanes\focus_research_decisions_depth_live\tools\watch_focus_pause_fixture.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_candidates.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_static_contracts.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_decision_target_save_oracle.py replay_native\lanes\focus_research_decisions_depth_live\tools\probe_decision_numeric_target_save_oracle.py
```

Result: passed.

```powershell
cargo test --manifest-path replay_native\lanes\focus_research_decisions_depth_live\Cargo.toml
```

Result: `10 passed; 0 failed`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v5 --sample-limit-per-list 32
```

Result: `ok=true`, `valid_country_rows=439`, `focus_status_valid_rows=439`,
`active_targeted_timed_decision.records_sampled=1891`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_current_roots.py --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions --sample-limit-per-list 10000
```

Result: `ok=true`, `valid_country_rows=439`, `focus_status_valid_rows=439`,
`active_targeted_timed_decision.records_sampled=3384`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_candidates.py --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2
```

Result: `ok=true`, `focus_status_rows=439`, rejected byte candidates `24`,
direct byte-shape rejections `13`, structural overlap rejections `11`,
inconclusive offsets `0`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_wide_candidates.py --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2 --start-offset 0x0 --end-offset 0x80
```

Result: `ok=true`, `focus_status_rows=439`, scan range
`CFocusStatus+0x0..+0x7F`, excluded prior window `CFocusStatus+0x20..+0x37`,
outside-window rejected byte candidates `100`, known-field rejections `44`,
direct byte-shape rejections `39`, structural qword rejections `6`, structural
dword-scalar rejections `11`, outside-window inconclusive offsets `4`
(`0x3C..0x3F`).

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_focus_pause_static_gap_candidates.py --wide-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2\focus_pause_wide_candidate_gate.json --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_static_gap_gate_20260701_v2
```

Result: `ok=true`, `vtable_rva=0x2723D30`, `vtable_entries=28`,
`methods_scanned=18`, `methods_with_target_qword_access=2`,
`methods_with_target_gap_direct_access=0`, rejected offsets
`0x3C`, `0x3D`, `0x3E`, `0x3F`, remaining inconclusive offsets `0`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\watch_focus_pause_fixture.py --focus-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2\focus_pause_candidate_gate.json --duration-s 12 --sample-ms 500 --min-game-hours 2 --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_fixture_watch_20260701_v1
```

Result: `ok=true`, `fixture_status=BLOCKED_NO_PAUSED_TRUE_ORACLE`,
`unique_game_hour_count=2`, `focus_candidate_window_rejected=true`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_body_candidates.py --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_body_candidate_gate_20260701_v3_full_decisions
```

Result: `ok=true`, `rejected_candidate_count=22`, `remaining_blocker_count=1`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_decision_variant_candidates.py --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions --with-static-defs --out-dir replay_native\runs\focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions
```

Result: `ok=true`, `records_sampled=3384`, `decision_token_count=262`,
`candidate_slices=50`, `rejected_candidate_slices=49`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_decision_target_save_oracle.py --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_target_save_oracle_gate_20260701_v1
```

Result: `ok=true`, `validated_field=active_targeted_timed_decision 0x27D5038
record+0x20 country_target_tag_db_id`, `live_rows=3384`,
`validated_live_country_target_rows=1061`, `validated_decision_count=180`,
`unmatched_live_country_target_rows=33`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_decision_numeric_target_save_oracle.py --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_numeric_target_save_oracle_gate_20260701_v1
```

Result: `ok=true`, `validated_field=active_targeted_timed_decision 0x27D5038
record+0x24 numeric_target_id`, `save_numeric_target_rows=2288`,
`validated_live_numeric_target_rows=2285`, `matched_intersection_ratio=0.998689`,
`unmatched_live_numeric_target_rows=4`, `unmatched_save_numeric_target_rows=3`,
`runner_up_intersection_rows=7`.

The same `cargo test` run also proves the lane-local compact schema/decoder/gate
for these target fields:

- `derives_targeted_timed_target_rows_from_extended_decisions_payload` accepts
  one validated country target and one validated numeric target out of two
  targeted-timed body rows.
- `depth_gate_rejects_bad_decision_country_target_row` rejects an invalid target
  tag and missing `record+0x20` provenance flag.
- `decisions_payload_rejects_bad_target_extension_flags` rejects malformed
  future `decisions_live` target-extension flags before derivation.
- `depth_payload_round_trip_gates_transition_rows` verifies target-row encoding
  in the depth payload next to focus/research/decision/badge rows.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_candidates.py --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1
```

Result: `ok=true`, `rejected_candidates=2`,
`possible_static_anchor_count=5`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\probe_event_hook_static_contracts.py --event-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1\event_hook_candidate_gate.json --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_static_contract_gate_20260701_v1
```

Result: `ok=true`, `candidate_anchor_count=5`,
`rejected_static_anchor_count=5`, `remaining_static_anchor_count=0`.

```powershell
python replay_native\lanes\focus_research_decisions_depth_live\tools\analyze_residual_blockers.py --probe-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\current_roots_20260701_v6_full_decisions --depth-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\integrated_depth_gate_20260701_v1\depth_gate_manifest.json --integrated-manifest replay_native\runs\focus_research_decisions_depth_live_integrated_870ebf_20260701_v1\native_session_manifest.json --focus-pause-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_candidate_gate_20260701_v2\focus_pause_candidate_gate.json --focus-pause-watch replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_fixture_watch_20260701_v1\focus_pause_fixture_watch.json --focus-pause-wide-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_wide_candidate_gate_20260701_v2\focus_pause_wide_candidate_gate.json --focus-pause-static-gap-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\focus_pause_static_gap_gate_20260701_v2\focus_pause_static_gap_gate.json --event-hook-static-contract-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_static_contract_gate_20260701_v1\event_hook_static_contract_gate.json --completed-depth-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\exact_completed_gate_20260701_v2\depth_gate_manifest.json --completed-integrated-manifest replay_native\runs\focus_research_decisions_depth_live_exact_completed_20260701_v1\native_session_manifest.json --decision-body-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_body_candidate_gate_20260701_v3_full_decisions\decision_body_candidate_gate.json --decision-variant-gate replay_native\runs\focus_research_decisions_depth_live_decision_variant_gate_20260701_v3_full_decisions\decision_variant_candidate_gate.json --decision-target-save-oracle-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_target_save_oracle_gate_20260701_v1\decision_target_save_oracle_gate.json --decision-numeric-target-save-oracle-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\decision_numeric_target_save_oracle_gate_20260701_v1\decision_numeric_target_save_oracle_gate.json --event-hook-gate replay_native\lanes\focus_research_decisions_depth_live\proofs\event_hook_candidate_gate_20260701_v1\event_hook_candidate_gate.json --decision-target-compact-schema-proved --integrated-replay-verified --completed-replay-verified --out-dir replay_native\lanes\focus_research_decisions_depth_live\proofs\residual_blocker_gate_20260701_v10
```

Result: `ok=true`, `completion_status=NOT COMPLETE`, `validated_slices=4`,
`rejected_candidates=7`, `remaining_blockers=3`.
