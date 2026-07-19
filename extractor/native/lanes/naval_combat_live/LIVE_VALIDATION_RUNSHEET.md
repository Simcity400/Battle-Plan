# Naval Combat Live Validation Runsheet

Purpose: validate the 870ebf naval combat lifecycle/member chain during a live
naval battle using the CFEXGroup back-ref member walk. `front+0xA4` is the
PROMOTED battle location (validated 4/4 against the same-tick D04 1944-02-11
save oracle); every validated combat must emit compact `location_id` plus the
static `region_id` join.

## Preconditions

- the game is running build sha256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- At least one active naval battle exists, preferably fleet-vs-fleet with
  non-empty attacker and defender ship rows (convoy-defender battles also
  validate; convoy members carry packed handle 0).
- The game install (module_path parent) has `map/strategicregions` available
  for the province -> strategic-region join (pass `--game-root` to override).

## Commands

Run from the repository root:

```powershell
$repo = ''
Set-Location $repo
$hoi4Pid = (Get-Process the game | Select-Object -First 1 -ExpandProperty Id)
$run = 'extractor\native\runs\naval_combat_live_active_fixture_harness_870ebf_<fixture>'
python extractor\native\lanes\naval_combat_live\tools\active_fixture_proof.py --pid $hoi4Pid --workers 4 --timeout-s 1800 --poll-s 5 --require-two-sided --require-cship-join --out-dir $run
```

Save these outputs from `$run`:

- `active_fixture_harness_summary.json`
- `poll_*.json`
- `active_fixture_input.json`
- `rust_fixture_proof\naval_combat_live_lane_stack_proof.json`
- `rust_fixture_proof\naval_combat_live_domain49_50_gate.json`
- `rust_fixture_proof\native_session.battleplan`
- `bridge.txt` and `member_bridge.jsonl` only when status is `passed`
- any `*.REJECTED.*` files if the required CShip join fails or Rust proof fails

## Success Criteria

- Python exits `0`.
- `active_fixture_harness_summary.json.status == "passed"`.
- Final fixture report has `validated_combat_count > 0`.
- `bridge_row_count > 0`.
- `member_bridge_row_count >= 2`.
- `cship_unresolved_member_count == 0`.
- `poll_*.json.vtable_rtti.ok == true` (all pinned vtable RVAs re-verified by
  RTTI class name at attach).
- No combat reports `forward_walk_found_group_missing_from_backref_scan`.
- Rust proof status is `passed`.
- Rust gate has lifecycle rows and member rows,
  `lifecycle_location_row_count == lifecycle_rows`,
  `lifecycle_region_row_count == lifecycle_rows`, and
  `location_candidate_unvalidated == false` (the unvalidated-candidate status
  flag is clear because location/region are promoted and present).
- `active_fixture_input.json.source.root.sha256_ok == true` and
  `source.root.sha256` equals the pinned 870ebf SHA.
- `active_fixture_input.json.source.location_front_0xA4.validated == true` and
  every row carries a non-null `location_front_0xA4` and `region_id`.

## Location Provenance

`front+0xA4` was validated on 2026-07-03 against the same-tick melted save of
the paused D04 fixture: live value == save `naval_combat.location` for all 4
battles (6434, 2804, 1032, 8819), with live `combat_id` (body+0x18 high u32)
matching the save `naval_combat.id.id` for each. Evidence:
`extractor/native/runs/nav49_50_port_validation/`. `region_id` is a static
province -> strategic-region join over `map/strategicregions/*.txt`; an
unmapped province is a fail-closed fixture error.

## Failure Criteria

- Build SHA mismatch or `source.root.sha256_ok != true`.
- `vtable_rtti_mismatch`: any pinned vtable RVA fails the attach-time RTTI
  class-name check (includes the `CProvince` location object). Terminal,
  exits nonzero.
- `scan_truncated`: a body/group vtable scan exceeded its hit cap — a
  truncated hit list could silently drop live battles/groups. Terminal.
- `bad_front_province_vtable` / `missing_front_location_province` on a combat:
  `body+0x38` did not point at a vtable-verified `CProvince`, or its province
  id read as 0. The combat fails closed (no plausible-but-unverified location).
- `duplicate_ship_instance_id_across_member_groups` on a combat: the same ship
  appeared twice in one battle — stale/duplicated CFEXGroup; fails closed.
- Any validated combat missing `location_front_0xA4` or its region join: the
  fixture enrichment fails closed (`validated_fixture_enrichment_failed`).
- Required CShip join failure: status `candidate_rejected_cship_member_join` or
  `validated_member_rows_missing_cship_fixture`. This must exit nonzero and must
  not produce normal `bridge.txt` or `member_bridge.jsonl`; only `.REJECTED`
  diagnostics are allowed.
- Rust fixture proof accepts fixture JSON from any build other than the pinned
  870ebf SHA.
- Rust compact output missing a lifecycle location/region row for any active
  combat, or carrying `STATUS_LOCATION_CANDIDATE_UNVALIDATED`.
- Any normal bridge artifact is written for a failed required join.
