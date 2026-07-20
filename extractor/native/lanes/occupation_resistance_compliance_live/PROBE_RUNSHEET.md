# Garrison Probe Runsheet

Lane: `occupation_resistance_compliance_live`

Historical status: the scalar-discovery steps in this runsheet are superseded
by `GARRISON_RESIDUAL_ANALYSIS.md`. The production fields
`army_manpower_need`, `strength_ratio`, and `garrison_required` are closed. Use
this procedure only when collecting new evidence for the still-separate
manpower-pool, equipment-need, suppression, or reinforcement-request details;
do not reopen the three scalar offsets from these commands.

Purpose: next live-session validation for the master-contract `Garrisons` row.
This runsheet stops at discovery evidence. Do not wire or promote any new
garrison scalar/detail root from these commands alone.

## Fixed Gates

- Process: `hoi4.exe`
- Required SHA-256:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
- Python path:

```powershell
$env:PYTHONPATH = "extractor/memory_reader"
```

Every live probe below writes a fail-closed JSON report and exits nonzero if
`hoi4.exe` is absent or the SHA mismatches.

Use a gitignored run directory. Replace `<STAMP>` and
`<ABS_MELTED_1943_IRE_SAVE>` before running:

```powershell
$run = "extractor/native/runs/occupation_garrisons_870ebf_<STAMP>"
New-Item -ItemType Directory -Force $run
```

## Step 1: Build Save Oracle

```powershell
python extractor/memory_reader/occupation/build_garrison_oracle.py `
  --save-melt "<ABS_MELTED_1943_IRE_SAVE>" `
  --out "$run/garrison_oracle.json" `
  --summary "$run/garrison_oracle_SUMMARY.md" `
  --expect-date 1943.5.1.1
```

Expected output:

- `garrison_oracle.json`
- `garrison_oracle_SUMMARY.md`
- stdout JSON with `state_garrison_rows`, `active_occupiers`, and save date.

Success criteria:

- Save date is exactly `1943.5.1.1`.
- Oracle has nonzero `state_garrison_rows`.
- Oracle records nonzero `rows_with_manpower_pool` and/or
  `rows_with_equipment_need`.

Failure criteria:

- Date gate fails.
- `state_garrison_rows = 0`.
- The melted save is not the 1943 IRE fixture.

## Step 2: Paused Static Probe

Run while the 1943 IRE save is loaded and paused.

```powershell
python extractor/memory_reader/occupation/probe_garrison_candidates_870ebf.py `
  --oracle "$run/garrison_oracle.json" `
  --out "$run/garrison_candidate_probe_paused.json" `
  --snapshot-label paused_static_1943_ire `
  --max-rows 120 `
  --graph-countries 60
```

Expected output:

- `garrison_candidate_probe_paused.json`
- `fail_closed = false`
- `process.sha256` equals the required SHA.
- `oracle_compare.same_law_matches` is nonzero.
- `object_graph.verdict` is
  `validated_per_state_law_join_roots_unclassified_garrison_scalars` or a
  more specific non-failing successor.

Candidate success criteria:

- `CCountryOccupationStatus+0x78` template rows remain populated through the
  validated status root.
- `CStateGarrisonData+0x50/+0x58` joins validate for occupied states.
- Same-offset law/garrison pairs still join per-state law assignments.
- For any scalar/detail field, exactly one candidate scope crosses threshold
  and has no law mismatch.

Candidate failure criteria:

- SHA mismatch or process missing.
- No same-law oracle matches.
- Object graph does not validate state-garrison rows.
- A field has zero threshold scopes: keep unavailable.
- A field has multiple threshold scopes: fixture remains ambiguous.
- A candidate is one of the already rejected offsets without new proof that
  directly overturns the old rejection.

## Step 3: UI Oracle Fallback

If the paused static probe does not produce a unique candidate, fill this table
from the Occupied Territories screen. Use states visible in
`garrison_candidate_probe_paused.json`, preferably states with distinct laws or
non-default requirements.

Save as `$run/garrison_ui_oracle.csv`:

```csv
state_id,garrison_manpower_demand,garrison_equipment_demand,suppression_ratio,notes
0,,,,replace with real state rows from the UI
0,,,,
0,,,,
0,,,,
0,,,,
```

Then rerun the paused probe with UI evidence:

```powershell
python extractor/memory_reader/occupation/probe_garrison_candidates_870ebf.py `
  --oracle "$run/garrison_oracle.json" `
  --ui-oracle-csv "$run/garrison_ui_oracle.csv" `
  --out "$run/garrison_candidate_probe_paused_ui.json" `
  --snapshot-label paused_static_1943_ire_ui_oracle `
  --max-rows 120 `
  --graph-countries 60
```

UI success criteria:

- UI rows refer to states present in the live garrison dump.
- A candidate exactish match appears for a UI field and does not conflict with
  the save-oracle field checks.
- The candidate is treated as evidence only, not a promoted offset.

UI failure criteria:

- UI state ids are absent from the live garrison dump.
- Every candidate has zero matches.
- Multiple offsets match equally.
- The UI value was transcribed from a total/country row rather than a per-state
  row.

## Step 4: Short Unpause Dynamic Probe

Suppression and drain fields may need movement. From the same paused session.

`--ui-oracle-csv` is OPTIONAL for this before-unpause snapshot: pass
`$run/garrison_ui_oracle.csv` ONLY if you produced it in Step 3; if Step 3 was
skipped (the static probe already gave a unique candidate), omit the
`--ui-oracle-csv` line entirely (the probe treats an absent UI oracle as empty
and runs on the save-oracle checks alone).

```powershell
python extractor/memory_reader/occupation/probe_garrison_candidates_870ebf.py `
  --oracle "$run/garrison_oracle.json" `
  --out "$run/garrison_candidate_probe_before_unpause.json" `
  --snapshot-label before_short_unpause `
  --max-rows 120 `
  --graph-countries 60
# If Step 3 ran, add:  --ui-oracle-csv "$run/garrison_ui_oracle.csv"
```

Operator step:

- Unpause briefly, preferably 6 to 24 in-game hours.
- Do not change occupation law, garrison template, or selected country.
- Pause again after the short window.
- Always create `$run/garrison_ui_oracle_after.csv` before the after probe.
  Use the same CSV format as the before file:
  `state_id,garrison_manpower_demand,garrison_equipment_demand,suppression_ratio,notes`.
  If nothing visibly changed, copy the before rows unchanged and note that in
  `notes`. If values changed, update only the affected per-state values.

```powershell
python extractor/memory_reader/occupation/probe_garrison_candidates_870ebf.py `
  --oracle "$run/garrison_oracle.json" `
  --ui-oracle-csv "$run/garrison_ui_oracle_after.csv" `
  --out "$run/garrison_candidate_probe_after_unpause.json" `
  --snapshot-label after_short_unpause `
  --max-rows 120 `
  --graph-countries 60
```

Dynamic success criteria:

- Both before/after probes pass the SHA gate.
- Same state keys survive across both probes.
- Any candidate value movement aligns with the UI/save meaning being tested.
- No unrelated law/template/control change invalidates the comparison.

Dynamic failure criteria:

- Game state changes occupation law/template/control during the window.
- Candidate values move broadly without matching the selected UI/drain signal.
- No target UI values move and the candidate remains static.
- Same state keys cannot be joined between before/after outputs.

## Per-Candidate Disposition Rules

`CCountryOccupationStatus+0x78` garrison template:

- Accept as still valid only if the paused probe preserves nonzero template rows
  through validated occupation-status roots.
- Reject a current run if the vtable/type/id checks fail or the template rows
  disappear despite occupied states.

`CCountryOccupationStatus+0x80` garrison template:

- Keep rejected unless a new probe explicitly compares it to the save oracle and
  overturns the previous `99` mismatches.

`CStateGarrisonData+0x48` country identity:

- Keep rejected unless a new object-graph proof shows controller or occupied
  tag matches. Prior proof had `0` controller and `0` occupied matches.

Inline scalar/detail offsets:

- Accept only as candidates when a single scope crosses threshold.
- Reject for the current fixture when all non-default oracle rows miss or the
  match ratio stays below threshold.
- Leave blocked when the fixture is ambiguous or has too little variance.

Direct-child/root offsets:

- Accept only as candidates when the child root is reachable from a validated
  garrison row and exactly one child scope matches the oracle/UI field.
- Reject only within the scanned `CStateGarrisonData+0x0..0x17F` direct-child
  scope and sampled `0x300` child window.

Manpower/equipment vectors:

- Do not treat a single-tag manpower pool matching the controller tag as proof
  of an independent manpower vector.
- Require either a multi-tag/non-default fixture or UI-backed one-variable
  movement before wiring.

## Stop Condition

Stop after the JSON outputs and this runsheet's criteria are evaluated. The
next worker/orchestrator may decide whether any candidate deserves a separate
production-root validation pass. Do not mark the `Garrisons` row closed from
this discovery run alone.
