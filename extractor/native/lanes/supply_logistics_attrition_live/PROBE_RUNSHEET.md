# Domain 44 Wave-2 Probe Runsheet

Lane: `supply_logistics_attrition_live`  
Build gate: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`  
Process: `hoi4.exe`  
Scope: discovery only. No command below promotes a root or removes a wall badge.

## Evidence And Rejection History

Read first in this lane:

- `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
- `extractor/native/lanes/supply_logistics_attrition_live/README.md`
- `extractor/documents/archive/native_lanes/supply_logistics_attrition_live/RESIDUAL_FIXTURE_CONTRACTS.md`

Carried-forward candidates:

- Exact tooltip: `CSupplySystem+0x10` rows are already decoded as bounded low-`i32` `/100000` province inputs. They are not exact remaining supply. Current CProvince probing narrowed exact-value candidates to `CProvince+0x2C8` two-deref leaves at second offsets `+0x218`, `+0x230`, `+0x2F8`, `+0x310`, `+0x348`, `+0x3D8`, and `+0x428`; all require same-frame tooltip anchors and a temporal/network differential.
- Stored disruption: a729 serializer/parser evidence proves a robin-hood table rooted at `CCountrySupplySystem+0x88` (bucket pointer `+0x90`, occupied count `+0x98`, mask `+0x9C`, sentinel/displacement byte `+0xA0`, `0x18` bucket stride). Bucket `+0x08` is the `(province,node_type)` key and `+0x10` is signed `i64/100000`. Retained exact anchors are D04 `(7081,3)=4017` and FRA `(911,3)=22173`. `+0x1E0` is instead the exact foreign-homebase vector. Neither field may be promoted until a bounded production root replaces the RTTI heap sweep.
- Attrition: existing division-body scalar scan found no production-safe attrition flag/reason without a same-frame visible/saved attrition oracle.

Proof-backed rejections that must not be retried without new evidence:

- Stale exact-supply chain `CProvince+0x190 -> +0x348 -> i32@+0x108` from build `16022851...` is rejected on `870ebf`; `+0x190` resolves to a module pointer in the current sample, not a heap leaf.
- Stale `CCountrySupplySystem` vtables `module+0x297B2F8/+0x297B348` and bounded `+0x100F0` translation guesses have zero current-build instances.
- `CCountrySupplySystem+0x20` node records are valid `0x30`-stride country/node/tag records, but record `+0x04` is rejected as exact motorization level (`21` matches, `96` mismatches). The best live motorization candidate `u8@record+0x2D` still mismatched `20/117`; the save oracle was single-level only.
- CSupply sibling arrays `+0x138` and `+0x150` remain rejected under current bounded low-`i32` guards: invalid/negative/out-of-range values or short reads. They require semantic anchors before reconsideration.
- The prior shared CSupply run decoded `13,396` rows and no failure flags, but `game_hours_advanced=0`; it is not production proof.

## Setup

Use one PowerShell session from repo root:

```powershell
$env:PYTHONPATH = "extractor\memory_reader"
$PID_HOI4 = <hoi4.exe pid>
$RUN = "extractor\native\runs\supply_logistics_attrition_wave2_870ebf_20260703_v1"
New-Item -ItemType Directory -Force $RUN | Out-Null
```

Every probe below opens `hoi4.exe`, computes the executable SHA, requires the exact build above, and writes fail-closed JSON on mismatch. If any probe reports `build_sha_mismatch`, stop and do not use the output.

Compile check before the live session:

```powershell
python -m py_compile extractor\memory_reader\supply\tooltip_parity_probe.py extractor\memory_reader\supply\pressure_sibling_probe.py extractor\memory_reader\supply\attrition_oracle_probe.py
```

## 1. Tooltip Parity Fixture

Keep the game paused on one frame. Open supply map mode and transcribe these exact province IDs:

```text
1, 100, 500, 1000, 2000, 3324, 3544, 6421, 9244, 12000, 13413
```

For each province tooltip, record only visible values:

- `remaining`: the top computed available/remaining supply value for the province.
- `hubs`: the line labelled supply from hubs / hub supply.
- `state`: the line labelled state/local state supply.
- `sum`: displayed total/sum if the tooltip shows it separately; otherwise repeat `remaining`.
- `source_ref`: screenshot or OCR path.

Write:

```powershell
notepad "$RUN\tooltip_anchors_live.json"
```

Shape:

```json
{
  "contract": "domain44 exact province supply tooltip anchors",
  "exe_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
  "game_date_hour": "YYYY.M.D.H",
  "source": "supply map tooltip same-frame screenshots/OCR",
  "anchors": [
    {"province_id": 3544, "remaining": 19.34, "hubs": 16.94, "state": 2.4, "sum": 19.34, "source_ref": "path"}
  ]
}
```

Capture same-frame domain-44 payload and CProvince candidates:

```powershell
python extractor\native\lanes\supply_logistics_attrition_live\live_probe.py --pid $PID_HOI4 --out-dir "$RUN\tooltip_live_probe"
python extractor\native\lanes\supply_logistics_attrition_live\cprovince_residual_probe.py --pid $PID_HOI4 --anchors "$RUN\tooltip_anchors_live.json" --out-dir "$RUN\tooltip_cprovince_probe"
python extractor\memory_reader\supply\tooltip_parity_probe.py --pid $PID_HOI4 --payload "$RUN\tooltip_live_probe\domain44_payload.bin" --anchors "$RUN\tooltip_anchors_live.json" --cprovince-json "$RUN\tooltip_cprovince_probe\cprovince_residual_probe.json" --out "$RUN\tooltip_parity_probe.json"
```

Expected outputs:

- `$RUN\tooltip_live_probe\domain44_payload.bin`
- `$RUN\tooltip_cprovince_probe\cprovince_residual_probe.json`
- `$RUN\tooltip_parity_probe.json`
- `$RUN\tooltip_parity_probe.md`

Success criteria:

- all three reports show the expected SHA;
- at least `8` anchors are comparable;
- `tooltip_parity_probe.json.status` is either `tooltip_formula_unresolved_unextracted_terms_likely` or `tooltip_candidate_matched_requires_temporal_differential`;
- no exact field is promoted. A matching candidate only advances to the next fixture if every anchor matches `remaining` or `sum` and `promotion_safe_exact_supply_fields=0`.

Failure criteria:

- fewer than `8` usable anchors;
- any value was guessed instead of transcribed;
- a candidate matches only `hubs`, `state`, or a partial subset;
- `cprovince_residual_probe` reports stale-chain reuse or build mismatch.

Next differential if a candidate matches all anchors: change one supply-network input on the same front, capture a second anchor file with the same province IDs, rerun the three commands into `$RUN\tooltip_diff2_*`, and require the same chain/formula to track `remaining` while rejecting `hubs`, `state`, and `sum` aliases.

## 2. Foreign, Convoy, Port, And Disrupted Pressure Fixture

Create a disrupted-supply situation before this step, for example an island garrison whose convoy route is blockaded. The save/UI fixture must expose at least one disrupted-supply anchor. Save the same loaded game state and parse supply:

```powershell
$SAVE = "<same-frame disrupted-supply save path>"
python extractor\save_oracle\parse_supply.py $SAVE --out-dir "$RUN\save_oracle_disrupted"
python extractor\native\lanes\supply_logistics_attrition_live\save_supply_oracle_summary.py "$RUN\save_oracle_disrupted\supply.json" --out-dir "$RUN\save_oracle_disrupted"
```

Run the known foreign-homebase control and sibling enumeration:

```powershell
python extractor\native\lanes\supply_logistics_attrition_live\foreign_homebase_pressure_record_probe.py --pid $PID_HOI4 --supply-json "$RUN\save_oracle_disrupted\supply.json" --out-dir "$RUN\pressure_foreign_control"
python extractor\memory_reader\supply\pressure_sibling_probe.py --pid $PID_HOI4 --supply-json "$RUN\save_oracle_disrupted\supply.json" --out "$RUN\pressure_sibling_probe.json"
```

Expected outputs:

- `$RUN\save_oracle_disrupted\supply.json`
- `$RUN\save_oracle_disrupted\supply_oracle_summary.json`
- `$RUN\pressure_foreign_control\foreign_homebase_pressure_record_probe.json`
- `$RUN\pressure_sibling_probe.json`
- `$RUN\pressure_sibling_probe.md`

Success criteria:

- `supply_oracle_summary.json.disrupted_supply_entries > 0`;
- pressure sibling report has `status=pressure_sibling_candidates_enumerated`;
- `ccountry_supply_system.joined_objects > 0`;
- `known_control_candidate.matched_in_this_run=true` if foreign-homebase anchors are present;
- `candidate_verdicts` contains at least one object with `category="disrupted_supply"`, `verdict="PASS"`, and `oracle_match_count >= threshold`;
- every pressure candidate listed in `aggregated_candidates` has `candidate_verdicts[*].reason` entries explaining each PASS/FAIL category decision;
- `promotion_safe_pressure_rows=0`.

Failure criteria:

- zero disrupted anchors: fixture failed, do not interpret sibling misses as rejections;
- tag join missing for the disrupted country: document as join blocker, not candidate rejection;
- no `candidate_verdicts` entry has `category="disrupted_supply"` and `verdict="PASS"`: disrupted root remains open for this fixture;
- only foreign-homebase control verdicts pass and no disrupted/convoy/port category verdict passes: convoy/port/disrupted roots remain open.

## 3. Attrition Reason / Flag Fixture

Create or load a same-frame division attrition fixture. The UI oracle must include at least one visible attrition division and two nearby/control divisions without an attrition icon. Do not infer reasons; transcribe only visible UI text.

Write:

```powershell
notepad "$RUN\attrition_ui_oracle.json"
```

Shape:

```json
{
  "exe_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
  "game_date_hour": "YYYY.M.D.H",
  "source": "division tooltip/screenshots on same paused frame",
  "positive_divisions": [
    {"division_id": 12345, "owner_tag": "IRE", "province_id": 3544, "display_name": "1st Division", "reason": "visible attrition reason text", "source_ref": "path"}
  ],
  "negative_divisions": [
    {"division_id": 22345, "owner_tag": "IRE", "province_id": 3544, "display_name": "control A", "source_ref": "path"},
    {"division_id": 32345, "owner_tag": "IRE", "province_id": 3545, "display_name": "control B", "source_ref": "path"}
  ]
}
```

Run:

```powershell
python extractor\native\lanes\supply_logistics_attrition_live\live_probe.py --pid $PID_HOI4 --out-dir "$RUN\attrition_live_probe"
python extractor\memory_reader\supply\attrition_oracle_probe.py --pid $PID_HOI4 --ui-oracle "$RUN\attrition_ui_oracle.json" --payload "$RUN\attrition_live_probe\domain44_payload.bin" --supply-json "$RUN\save_oracle_disrupted\supply.json" --out "$RUN\attrition_oracle_probe.json"
```

Expected outputs:

- `$RUN\attrition_live_probe\domain44_payload.bin`
- `$RUN\attrition_oracle_probe.json`
- `$RUN\attrition_oracle_probe.md`

Success criteria:

- UI oracle has at least one positive and two negative controls;
- all oracle division ids resolve to live division bodies;
- report has either `attrition_candidates_require_fixture_diff` or `attrition_oracle_rejected_simple_division_scalars`;
- every candidate offset is listed with positive/negative values and alias verdict;
- `promotion_safe_attrition_fields=0`.

Failure criteria:

- division ids cannot be resolved to live bodies;
- positive divisions have no visible reason text or no screenshot/OCR source;
- candidates alias current supply, supply deficit, fuel, province, owner, movement/combat/template context, or cached UI state.

If a candidate separates positives and negatives, rerun before/after a controlled reason change for the same division id. The candidate must change with the visible reason while negative controls remain stable before it can become anything more than a candidate.

## 4. Shared CSupply Province-Inputs Clock-Advancing Rerun

This closes only the fixture-blocked shared gate for `CSupplySystem` province inputs, not exact tooltip supply.

Run with the game actually unpaused/advancing at speed five or current equivalent:

```powershell
cargo build --manifest-path extractor\native\Cargo.toml --release -p collector -p recorder_host -p snapshot

$SHARED_RUN = Join-Path $RUN "shared_csupply_speed5"
New-Item -ItemType Directory -Force $SHARED_RUN | Out-Null

extractor\native\target\release\collector_record.exe `
  --pid $PID_HOI4 `
  --dll extractor\native\target\release\collector.dll `
  --out-dir $SHARED_RUN `
  --duration-s 90 `
  --sample-ms 0 `
  --min-game-hours 5 `
  --scheduled-domain supply_logistics_attrition_live `
  --stream-format compact `
  --extraction-perf-gate `
  --lightweight-root-frame

cargo run --manifest-path extractor\native\lanes\compact_replay_verification_gates\Cargo.toml --bin compact_replay_gate -- --run-dir $SHARED_RUN --require-domain supply_logistics_attrition_live --min-game-hours 5 --write-report "$SHARED_RUN\compact_gate_report.domain44.json"
extractor\native\target\release\pack_replay.exe verify "$SHARED_RUN\native_session.battleplan"
```

Success criteria:

- manifest has `game_hours_advanced >= 5`;
- manifest has `skipped_game_hours = 0`;
- domain 44 data units include `supply_csupply_input_rows > 0` and `supply_csupply_efficiency_rows > 0`;
- no parse/source/worker/bridge/partial failures;
- `.battleplan` verifies;
- first-frame domain 44 read no longer includes a multi-second heap scan.

Failure criteria:

- `game_hours_advanced=0` repeats the old blocker;
- any source/partial/worker/bridge failure appears;
- CSupply row counters are zero;
- replay verification fails.

## Stop Conditions

Stop after producing the JSON/Markdown reports above. Do not edit shared collector, recorder, compact contract, or replay interface files from this lane. Do not claim exact tooltip supply, convoy/port/disrupted pressure, attrition reason, or shared CSupply production completion until a later live run satisfies the success criteria and a serial integration owner accepts it.
