# Air Residuals Probe Runsheet

Purpose: next live-session commands for the air residuals discovery lane. All
probes are read-only polling scripts unless explicitly marked as outside polling.
Every script gates on `hoi4.exe` SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad` and fails
closed on mismatch.

## Global Setup

Run from repository root:

```powershell
$env:PYTHONPATH = "extractor/memory_reader"
```

the game process:

- Process name: `hoi4.exe`.
- Build SHA must match `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Do not run if the game is on another build.
- Do not use these debug probes as production acceptance. They produce evidence
  for the next extractor decision only.

Output roots below use `extractor/native/runs/`, which is gitignored and absent
from this worktree until the live orchestrator creates it.

## Fixture A: Active Air War Polling

Game state required:

- Load a 1943-or-later save with active strategic bombing plus fighter combat in
  one named strategic air region, preferably `Western Germany`.
- Keep at least two opposing countries active in that region.
- Record from the UI/save oracle before running:
  - region display name and strategic region id;
  - two or more wing ids in the region;
  - owner tag-db ids for those wings;
  - visible plane counts by owner/mission type;
  - current game date/hour.
- Let the game run unpaused during the probe at speed 3 to speed 5. The run
  needs at least one visible air-loss tick; if no losses occur, the probe is
  structural only.

Create a fixture JSON from the recorded UI/save oracle. Example path:

```text
extractor/native/runs/air_residuals_fixture_870ebf_v1/air_residuals_fixture.json
```

Required shape:

```json
{
  "fixture_name": "1943_active_air_war_western_germany",
  "expected_regions": [123],
  "expected_wings": [
    {"owner_tag_db_id": 1, "wing_id": 1001, "mission_region": 123}
  ],
  "expected_region_owner_counts": [
    {"mission_region": 123, "owner_tag_db_id": 1, "min_plane_count": 1}
  ]
}
```

Replace all numeric placeholders with live UI/save-oracle values.

Command:

```powershell
python extractor/memory_reader/air/air_rtti_polling_probe_870ebf.py --process hoi4.exe --duration-s 60 --interval-s 0.20 --max-wings 160 --body-span 0x480 --fixture-json extractor/native/runs/air_residuals_fixture_870ebf_v1/air_residuals_fixture.json --out extractor/native/runs/air_residuals_polling_870ebf_v1/air_rtti_polling_probe_870ebf.json
```

Expected output:

- `status = "completed_candidates_only"`.
- `root.sha256_ok = true`.
- `fixture_crosscheck.status = "passed"`.
- `rtti_rediscovery.classes` contains current-build rows for air classes such as
  `CAirMission`, `CAirRegionCombatData`, `CAirWing`, `CStrategicAir`, and
  related classes. Missing classes are evidence, not automatic rejection.
- `delta_summary.loss_like_delta_count > 0` for a useful attribution run.
- `delta_summary.unknown_u32_field_candidates` lists candidate offsets only.

Success criteria:

- The fixture cross-check passes against the known region/wing oracle.
- At least one loss-like delta is observed while game hours advance.
- Candidate fields, if any, have concrete before/after examples tied to wing
  keys and sample times.
- No candidate is promoted from this run alone.

Failure criteria:

- SHA mismatch or root validation failure: discard the run.
- Fixture cross-check fails: treat the save/UI oracle as stale or wrong; do not
  reject candidates.
- No loss-like deltas: structural RTTI and field inventory only; do not use it
  to reject kill/cause or sortie lifecycle candidates.
- Unknown field deltas appear only on known count/equipment/mission offsets or
  without repeatable same-fixture correlation: keep candidate blocked.

What this validates:

- Whether polling can expose wing-local counters/timers or mission/region object
  state that co-changes with active air combat.

What this cannot validate:

- Exact killer, loss cause/source, sortie start/end, or combat pairings unless a
  candidate field is later cross-checked against a same-session UI/save oracle
  and repeated under a controlled fixture.

## Fixture B: Ace Assignment Side Effects

Game state required:

- Use a save with an owner country that has at least one ace and at least one air
  wing eligible to receive that ace.
- Pause the game.
- Pick one owner tag, one named wing, and one named ace. Record:
  - owner tag, owner tag-db id if known;
  - wing id;
  - ace id, name/surname/callsign if known;
  - current game date/hour.

Before assignment command:

```powershell
python extractor/memory_reader/air/air_ace_assignment_fixture_probe_870ebf.py --process hoi4.exe --snapshot-label before_assign --owner-tag GER --wing-id <WING_ID> --ace-id <ACE_ID> --out extractor/native/runs/air_ace_assignment_870ebf_v1/before_assign.json
```

Operator action:

- In the UI, assign the named ace to the named wing.
- Do not change airbase, mission, wing composition, or game speed between
  snapshots.

After assignment command:

```powershell
python extractor/memory_reader/air/air_ace_assignment_fixture_probe_870ebf.py --process hoi4.exe --snapshot-label after_assign --owner-tag GER --wing-id <WING_ID> --ace-id <ACE_ID> --compare-before extractor/native/runs/air_ace_assignment_870ebf_v1/before_assign.json --out extractor/native/runs/air_ace_assignment_870ebf_v1/after_assign.json
```

Operator action:

- Remove the same ace from the same wing.
- Do not change unrelated air state.

After removal command:

```powershell
python extractor/memory_reader/air/air_ace_assignment_fixture_probe_870ebf.py --process hoi4.exe --snapshot-label after_remove --owner-tag GER --wing-id <WING_ID> --compare-before extractor/native/runs/air_ace_assignment_870ebf_v1/after_assign.json --out extractor/native/runs/air_ace_assignment_870ebf_v1/after_remove.json
```

Replace `GER`, `<WING_ID>`, and `<ACE_ID>` with the live fixture values.

Expected output:

- `status = "completed_snapshot"`.
- `root.sha256_ok = true`.
- `owner_container.available = true`.
- `owner_ace_roster.available = true`.
- `selected_wing.matched_rows = 1`.
- For assignment, `compare_before.status =
  "candidate_assignment_side_effect_observed"`,
  `compare_before.preconditions.owner_container.stable = true`,
  `compare_before.preconditions.owner_container_dump.stable = true`,
  `compare_before.preconditions.owner_ace_roster.readable = true`, and the
  after value at `body_assigned_ace_id_0x2DC` equals `<ACE_ID>`.
- For removal, the assignment field changes away from the assigned ace id.

Success criteria:

- The same wing key is present before/after.
- The selected owner strategic-air container is stable and dumped in both
  snapshots.
- The country ace roster is readable and contains the selected ace id.
- The selected wing assignment field changes exactly with the UI action.

Failure criteria:

- SHA mismatch or root validation failure: discard the run.
- Owner container missing: fixture target owner was wrong or root is not stable;
  do not reject ace assignment.
- Selected wing missing or duplicate: fixture wing id is wrong; rerun with a
  clean wing id.
- Assignment field does not change after the UI action: keep assignment payload
  blocked; do not reject `rdx` without proving the UI action actually dispatched
  command `0x2FB5`.

`rdx = r14` payload validation:

- The polling script cannot observe `rdx`; it only validates side effects.
- To validate the payload interpretation, a separately approved callsite
  register capture must hit `0x6FA8BA -> 0xC52840` during command id `0x2FB5`.
- Acceptance requires:
  - `rcx` equals the selected owner strategic-air container dumped by this
    runsheet.
  - `rdx/r14 + 0x30` discriminator matches one of the static branch values from
    `0xC52840`.
  - `rdx/r14 + 0x28/+0x34/+0x38/+0x44` values match the owner/wing/ace side
    effects proven by the before/after polling snapshots.
- Without that register capture, the result can validate polling-observable ace
  assignment state but cannot validate the command payload.

## Fixture C: Reinforcement Attribution Check

Use Fixture A output plus existing production domains.

Required replay-side inputs:

- Domain 30/32 air-wing identity/loadout rows.
- Domain 42 count/equipment deltas from the active fixture.
- Domain 46 variant archetype and ace roster rows if enabled.
- Domain 11 production/stockpile rows for the same replay window.

Validation task:

- For each positive D42 wing delta, compare the variant/equipment increase with
  same-window production/stockpile evidence from D11. If separately sourced
  domain 43 market/lend-lease delivery rows are present, badge them as optional
  delivery context; do not treat D11 as transfer evidence.

Success criteria:

- Exact facts can be emitted: wing key, time window, old/new count, equipment
  delta, variant ids, mission region/mask, country production/stockpile context,
  and separately badged delivery context only when supplied by a non-D11 source.

Failure / blocker criteria:

- If no root joins a wing reinforcement delta to a source transaction, keep
  `Reinforcement reason` as `ROOT_DISCOVERY_OPEN`.
- Replay-side inference may be badged as possible context only. Do not emit
  exact `production`, `lend_lease/market_delivery`, or `attrition replacement`
  cause labels.

## Files Produced By This Wave

- `AIR_RESIDUALS_STATE.md`: rejection synthesis and polling/reinforcement spec.
- `PROBE_RUNSHEET.md`: this live-session runsheet.
- `extractor/memory_reader/air/air_rtti_polling_probe_870ebf.py`: RTTI and
  polling candidate probe.
- `extractor/memory_reader/air/air_ace_assignment_fixture_probe_870ebf.py`: ace
  assignment before/after polling snapshot probe.
