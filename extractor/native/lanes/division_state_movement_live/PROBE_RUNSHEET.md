# Division Manpower And Assignment Probe Runsheet

Lane: `division_state_movement_live`
Related lane: `army_oob_orders_battleplans_live` for domain-54 order rows
Build gate: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
Process: `hoi4.exe`
Status: root discovery only. Do not promote manpower, losses, assignment, or
strategic redeploy fields from this runsheet without live JSON evidence.

## Non-Negotiable Rules

- Load the 1943 Ireland save in `hoi4.exe` and keep the game paused for static
  oracle-backed snapshots.
- The save parsed by the oracle must be the exact save loaded in the process.
- Replace placeholder paths such as `<REPO_ROOT>`, `<BATTLE_PLAN_USERDATA>`, and
  `<SAVE_NAME>.hoi4` before running any command.
- Every probe gates on the SHA above and fails closed on mismatch.
- Set `PYTHONPATH` to `extractor/memory_reader`.
- Output goes under gitignored `extractor/native/runs/...`.
- A shape match or loose heap pointer is candidate evidence only. Equality
  against a same-save oracle is required before a candidate can be carried
  forward, and a later live integration gate is still required before compact
  emission.

## Current Evidence Basis

Validated division enumeration for this build uses:

- `CGameState = u64[module+0x3316E90]`.
- Country table at `CGameState+0x310`, count at `CGameState+0x31C`.
- `CCountry` vtable `module+0x27D0E90`.
- Division wrapper vector `CCountry+0x290/+0x298/+0x29C`.
- Wrapper vtable `module+0x2943AD0`.
- `CDivision = u64[wrapper+0x10]`, vtable `module+0x2943CB0`.
- `CDivision+0x0C` is the division id.

Validated army/order context from the related lane:

- `CDivision+0xB0 -> CArmy*` is the positive-control parent-army route.
- `CArmy` vtable `module+0x293BC60`.
- Order vector at `CArmy+0x98`.
- Order vtable `module+0x296FAE8`, type at `+0x30`, army backref at `+0x38`.
- Type-5 order target-state vector at `order+0xE0/+0xE8`.

Current open gaps:

- Exact current manpower and cumulative losses are not pinned. Existing compact
  domain-20 rows include strength and organisation, but no exact manpower flag.
- Existing movement fields cover ordinary movement; strategic redeploy remains
  unvalidated and must not be inferred from movement progress alone.
- The residual assignment row needs division-to-army/order/battleplan linkage
  evidence beyond already-known army parent and order rows.

## Orchestrator Commands

Run from the repository root:

```powershell
Set-Location '<REPO_ROOT>'
$env:PYTHONPATH = (Resolve-Path 'extractor\memory_reader').Path
$runName = 'division_manpower_assignment_870ebf_' + (Get-Date -Format 'yyyyMMdd_HHmmss')
$runDir = Join-Path (Resolve-Path '.').Path "extractor\native\runs\$runName"
New-Item -ItemType Directory -Force -Path $runDir | Out-Null
```

Build the same-save oracle. Substitute the save placeholder with the exact 1943
Ireland save loaded in `hoi4.exe`. If the save is binary and `rakaly.exe` is
available, add `--rakaly '<RAKALY_EXE>'`.

```powershell
python extractor\memory_reader\divisions\division_save_oracle_870ebf.py `
  --save '<BATTLE_PLAN_USERDATA>\save games\<SAVE_NAME>.hoi4' `
  --out (Join-Path $runDir '00_division_save_oracle.json') `
  --melted-copy (Join-Path $runDir '00_1943_ire_melted.txt')
```

Static paused manpower/loss candidate scan:

```powershell
python extractor\memory_reader\divisions\probe_870ebf_division_manpower.py `
  --oracle (Join-Path $runDir '00_division_save_oracle.json') `
  --out (Join-Path $runDir '01_division_manpower_paused.json') `
  --snapshot-label 'paused_1943_ire'
```

Static paused assignment/order/SR candidate scan:

```powershell
python extractor\memory_reader\divisions\probe_870ebf_division_assignment.py `
  --oracle (Join-Path $runDir '00_division_save_oracle.json') `
  --out (Join-Path $runDir '02_division_assignment_paused.json') `
  --snapshot-label 'paused_1943_ire'
```

## Losses Two-Poll Fixture

Use this only when the oracle emits at least one exact land-combat fixture:
`combat_fixtures[0]` in `00_division_save_oracle.json`. That row names the
concrete battle/province and division control set for this run:

- Battle: `combat_fixtures[0].combat_id` at exact province id
  `combat_fixtures[0].location_province`.
- Attacker controls: the 2-3 entries in
  `combat_fixtures[0].attacker_watch[*]`.
- Defender controls: the 2-3 entries in
  `combat_fixtures[0].defender_watch[*]`.

Before polling, copy those exact ids/names from the JSON into the run log. If
`combat_fixtures` is empty, or the UI shows that exact battle has ended, record
the fixture as blocked; do not reject loss candidates.

First paused poll:

```powershell
python extractor\memory_reader\divisions\probe_870ebf_division_manpower.py `
  --oracle (Join-Path $runDir '00_division_save_oracle.json') `
  --out (Join-Path $runDir '03_division_losses_before.json') `
  --snapshot-label 'losses_before'
```

Operator action:

1. Unpause only while the exact `combat_fixtures[0]` battle is still visible.
2. Let the game advance exactly two in-game hours, or 60-90 real seconds at
   speed 4/5 if the in-game hour boundary is easier to miss.
3. Pause immediately.
4. Confirm the exact battle/province above continued during the interval and
   that at least one listed watch division remained assigned to that battle.

Second poll and compare:

```powershell
python extractor\memory_reader\divisions\probe_870ebf_division_manpower.py `
  --oracle (Join-Path $runDir '00_division_save_oracle.json') `
  --compare-before (Join-Path $runDir '03_division_losses_before.json') `
  --out (Join-Path $runDir '04_division_losses_after_compare.json') `
  --snapshot-label 'losses_after'
```

## Strategic Redeploy Fixture

The static assignment probe reports save keys containing `strategic`,
`redeploy`, `transport`, or `embark` when the save exposes such hints. If the
1943 Ireland save has no strategic redeploy rows, the result is
`fixture_blocked_no_save_strategic_redeploy_rows`, not a rejection.

Controlled SR validation needs a later run:

1. Pick one known live division from `02_division_assignment_paused.json`.
2. Record its owner/id, location, movement progress, and any scalar partitions.
3. Command strategic redeployment for that division in-game.
4. Pause after the command is accepted.
5. Rerun the assignment probe with a new snapshot label.
6. Accept only a field that changes for the commanded division and not for
   ordinary-moving/non-moving controls.

Existing movement-mode findings do not already name strategic redeploy.

## Expected Outputs

`00_division_save_oracle.json`:

- `expected_live_exe_sha256` equals the build gate above.
- `validation_status` is either `validated_expected_division_blocks` for a
  real division-bearing save or `legitimately_empty_no_division_blocks` for a
  fixture sample with no division blocks.
- `stats.division_rows > 0` for the 1943 live fixture.
- `stats.current_manpower_rows > 0`.
- `stats.current_need_divergent_rows > 0` is required to reject template-need
  aliases for current-manpower candidates.
- `stats.nonzero_cumulative_killed_rows > 0` is required for cumulative-loss
  candidate narrowing. If zero, loss validation is fixture-blocked.
- `stats.assignment_rows > 0` is required for army-assignment oracle
  comparison.
- `combat_fixtures[0]` must exist before the two-poll losses fixture can run.

`01_division_manpower_paused.json`:

- `build_gate_ok: true`.
- `enumeration_stats.division_rows > 0`.
- Candidate rows must be classified as `narrowed_current_manpower_candidate`,
  `narrowed_cumulative_losses_candidate`, `rejected`, or `fixture_blocked_*`.
- A current-manpower candidate is carry-forward evidence only if it has exact
  same-save matches, high exact fraction, and
  `exact_divergent_current_need_rows > 0`.
- A cumulative-loss candidate is carry-forward evidence only if it matches
  nonzero killed rows. Zero-only equality is fixture-blocked.

`02_division_assignment_paused.json`:

- `build_gate_ok: true`.
- `assignment_positive_control.status = "positive_control_exact"` for
  `CDivision+0xB0 -> CArmy*` against the save OOB member oracle.
- Division pointer-field candidates must name whether they target `CArmy` or a
  validated order object.
- Army member-vector candidates must show direct/wrapper division pointer rows;
  shape-only vectors are not enough.
- Strategic redeploy candidates must be tied to save hints or a controlled SR
  fixture. Static scalar clustering alone is not acceptance.

`04_division_losses_after_compare.json`:

- Build gate still true.
- The previous snapshot schema and live snapshot schema match.
- A losses candidate needs positive cumulative deltas during active combat.
- A current-manpower candidate should show plausible negative deltas for
  fighting divisions or remain unchanged for non-fighting controls.
- If combat did not advance or no relevant row changed, classify as fixture
  failure/blocker, not as a proof that no loss field exists.

## Rejection History To Preserve

- Current compact domain-20 path has no manpower flag, and reserved manpower
  row `+84` is zero; this rejects the current compact path only, not memory.
- Country manpower pools, template manpower requirements, and equipment-bank
  quantities are rejected source levels for exact current per-division manpower.
- `CDivision+0x1D0` controller equality is not enough for exact controller or
  assignment validation.
- Existing movement progress and movement source fields do not prove strategic
  redeploy state.
- `order+0x150` is rejected as `is_executing`.
- `order+0xC0/+0xC8` is rejected as the primary type-3 path vector.
- Plain pointer-table direct roots, targeted arena-wide handle refresh, and
  pure direct-root preflight cache shortcuts are rejected for the order root.
- `CArmy+0x1A4/+0x1A8/+0x1AC` are near-match scalar candidates only, not exact
  source fields.

## Stop Condition

Stop after producing the JSON artifacts and this evidence classification. Do
not claim any manpower, loss, assignment, or strategic redeploy root validated
until the live artifacts meet the success criteria above and a later lane gate
turns that evidence into an explicit integration packet.
