# battleplan geometry/execution fixture contract

This lane must not promote passive candidate scans as replay data. A candidate
offset is accepted only after it becomes either exact live evidence or exact
rejection evidence.

## Accepted Geometry Evidence

`non_type5_geometry[]` rows must include:

- `kind = "geometry"`;
- `status = "exact"`;
- positive `army_id`;
- nonnegative `slot_index`;
- non-type-5 `order_type`;
- positive `order_generation`;
- non-empty `validated_offsets[]`, where each entry has:
  - `name`;
  - `offset` in `+0x...` form;
  - `type`;
  - `evidence`;
- non-empty `field_values`, such as province path, state ids, root-front id, or
  line points.

The fixture must come from a DLL compact replay capture with `.battleplan`
verification, zero skipped hours, and no source/partial/bridge/worker failures.

## Accepted Execution Evidence

`battleplan_execution_state[]` rows must follow the same exact evidence shape,
with `kind = "execution"` and `field_values` carrying the decoded execution
state, activation bit/state, or explicitly pinned plan-value/power field.

Cached save-oracle `plan_value`, `our_power`, and `enemy_power` values are not
accepted as live execution state unless a live offset is pinned and a fixture
proves it changes with the execution state being tested.

## Accepted Rejection Evidence

Rows may use `status = "rejected"` only when `battleplan_fixture_gate.py` is run
with `--allow-rejection`. Each rejected row must include:

- `rejection_evidence.candidate`;
- `rejection_evidence.reason`, one of:
  - `invalid_pointer`;
  - `mutation_mismatch`;
  - `not_execution_state`;
  - `not_geometry`;
  - `not_live_state`;
  - `stale_cached_value`;
- non-empty `rejection_evidence.evidence[]`.

## Explicit Non-Evidence

The following do not reduce the blocker:

- passive scans showing plausible province-like vectors;
- old-build offsets without current-build fixture evidence;
- save-only serialization ids;
- scheduled-member caches;
- cached save-oracle plan/power values without a live offset and liveness proof.

## Observation-To-Fixture Harness

`battleplan_fixture_observer.py` converts before/after observations into the
fixture JSON consumed by `battleplan_fixture_gate.py`.

Accepted observation rows must include:

- `operator_action`, listed in top-level `operator_actions`;
- stable `(army_id, slot_index, order_type, order_generation)`;
- `candidate` offset in `+0x...` form;
- `field_name` and `field_type`;
- `before.value` and `after.value`.

Changed before/after values become `status = "exact"` rows. Unchanged values
become exact rejection rows and require `--allow-rejection`. Rows with
`source_kind = "passive_scan"` fail immediately.

## Current Candidate Status, 2026-07-01

Live current-build candidate probe:
`extractor/native/runs/army_battleplans_depth_live_candidate_probe_870ebf_20260701_v2`.

The probe validated 350 current-build order objects after `CArmy` and order
vtable/back-reference gates:

- order types: `type=2` rows `172`, `type=3` rows `14`, `type=5` rows `164`;
- positive control `order+0xE0/+0xE8` type-5 state vector passed with `157/164`
  exact decodes and `7` fail-closed out-of-range rows;
- execution placeholder `order+0x150` as `is_executing: u8` is rejected:
  `7/350` rows read non-boolean values, with histogram
  `{0:343, 5:1, 6:1, 7:1, 9:1, 10:2, 11:1}`.

The following geometry candidates are narrowed but not validated:

- `order+0x70/+0x78` decoded vector-shaped values on `181/186` non-type-5 rows
  (`167/172` type-2, `14/14` type-3) and remains fixture-required;
- `order+0xC0/+0xC8` decoded vector-shaped values on `14/14` type-3 rows and
  remains fixture-required.

These vector-shaped candidates must not be promoted without mutation evidence.
They are candidate offsets for the redraw fixture, not production roots.

## Exact Redraw/Toggle Fixture

The next fixture must use a live DLL compact capture with domain 54 scheduled
after integration, or the lane-local before/after observer before integration.
The operator must perform both actions during the capture window:

1. Select an already-sampled army with a non-type-5 `type=2` front-line order.
   Redraw or resize the front line so its province path visibly changes.
2. Select an already-sampled army with a `type=3` fallback/front-related order,
   if available. Redraw or relocate it so its line/province path visibly
   changes.
3. Toggle battleplan execution for an already-sampled army from inactive to
   active, or active to inactive, without changing the selected order identity.

Required live evidence for geometry:

- stable `(army_id, slot_index, order_type, order_generation)` before and after;
- candidate `+0x70/+0x78` values before and after for the type-2 order;
- candidate `+0x70/+0x78` and `+0xC0/+0xC8` values before and after for the
  type-3 order when present;
- UI or save-oracle note identifying the intended line before and after, used
  only to label the operator action, not as hot-loop capture input.

Pass criteria for geometry:

- If a candidate changes exactly with the redraw while the stable order identity
  remains the same, emit an exact geometry row with validated offsets and the
  decoded field values.
- If a candidate remains unchanged or mutates independently of the redraw, emit
  a rejected row with `reason = "mutation_mismatch"` or `reason = "not_geometry"`.
- If no type-3 order exists in the fixture frame, the type-3 `+0xC0/+0xC8`
  candidate remains fixture-blocked, not rejected.

Required live evidence for execution:

- stable `(army_id, slot_index, order_type, order_generation)` before and after;
- candidate bytes/scalars for any proposed execution offset before and after;
- the already-rejected `+0x150 u8` path must not be retried as
  `is_executing`;
- UI action record: execution toggle changed state during capture.

Pass criteria for execution:

- A candidate is exact only if it changes with the execution toggle and does not
  change when only geometry is redrawn.
- A candidate is rejected if it is non-boolean for a boolean claim, unchanged
  across the toggle, or changes on unrelated geometry edits.
- Save-only `plan_value`, `our_power`, `enemy_power`, `instance_id`, and
  `scheduled_member` caches remain non-evidence unless a live offset is pinned
  and passes this before/after rule.
