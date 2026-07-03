# Naval Combat Live Validation Runsheet

Purpose: validate the 870ebf naval combat lifecycle/member chain during the
next live naval battle. This run must not promote location. `front+0xA4` remains
only `location_candidate_front_0xA4` until a later code change after manual
verification.

## Preconditions

- the game is running build sha256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- A visible active naval battle is on screen, preferably fleet-vs-fleet with
  non-empty attacker and defender ship rows.
- The operator can read the battle's displayed sea region/province from the UI
  or a trusted game-data lookup during the same paused/live moment.

## Commands

Run from the repository root:

```powershell
$repo = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\mod\battle-plan'
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
- Rust proof status is `passed`.
- Rust gate has lifecycle rows and member rows, `lifecycle_location_row_count == 0`,
  `lifecycle_region_row_count == 0`, and `location_candidate_unvalidated == true`.
- `active_fixture_input.json.source.root.sha256_ok == true` and
  `source.root.sha256` equals the pinned 870ebf SHA.

## Manual Location Candidate Check

Open `active_fixture_input.json` and record every
`source.location_candidate_front_0xA4.candidates[]` value. During the same live
fixture, compare each candidate against the on-screen battle location using the
visible battle region/province and trusted game data.

Expected current behavior:

- The candidate appears only under `source.location_candidate_front_0xA4`.
- Each candidate row has `"validated": false`.
- No compact lifecycle row carries `location_id` or `region_id`.

If the candidate matches the on-screen battle location, record the evidence in
the run directory and request a later code change to promote it. Do not promote
automatically in this run. If it does not match, keep location blocked and attach
the screenshot/game-data evidence to the run notes.

## Failure Criteria

- Build SHA mismatch or `source.root.sha256_ok != true`.
- Required CShip join failure: status `candidate_rejected_cship_member_join` or
  `validated_member_rows_missing_cship_fixture`. This must exit nonzero and must
  not produce normal `bridge.txt` or `member_bridge.jsonl`; only `.REJECTED`
  diagnostics are allowed.
- Rust fixture proof accepts fixture JSON from any build other than the pinned
  870ebf SHA.
- Rust compact output has any lifecycle location/region row before manual
  promotion.
- Any normal bridge artifact is written for a failed required join.
