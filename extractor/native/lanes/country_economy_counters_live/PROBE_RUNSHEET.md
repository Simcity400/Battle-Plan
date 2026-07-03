# Wave-2 Economy Residual Probe Runsheet

Lane: `country_economy_counters_live`  
Also covers residual coordination with `market_trade_live` and
`country_economy_production_stockpile_live`  
Process: `hoi4.exe`  
Required build SHA:
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

All probes below are discovery-only. They must fail closed on SHA mismatch and
must not be used to promote a root without same-frame oracle or controlled-delta
proof. Stop after collecting the JSON outputs and applying the criteria here.

## Common Setup

Run from repository root:

```powershell
$env:PYTHONPATH = "extractor/memory_reader"
$run = "extractor\native\runs\economy_residuals_wave2_870ebf_20260703"
New-Item -ItemType Directory -Force $run | Out-Null
$hoi4Pid = (Get-Process the game -ErrorAction Stop).Id
```

Expected: `$hoi4Pid` resolves to the live `hoi4.exe` process. If the game is not
running, stop. Do not substitute another process.

Compile gate:

```powershell
python -m py_compile `
  extractor\memory_reader\economy\offmap_factory_probe_870ebf.py `
  extractor\memory_reader\economy\resource_flow_delta_probe_870ebf.py `
  extractor\memory_reader\economy\market_license_rtti_probe_870ebf.py `
  extractor\native\lanes\country_economy_counters_live\verify_factory_totals_derive.py `
  extractor\native\lanes\country_economy_counters_live\build_country_economy_oracle.py `
  extractor\native\lanes\country_economy_counters_live\probe_870ebf_country_economy_roots.py `
  extractor\native\lanes\country_economy_counters_live\compare_870ebf_country_economy_probes.py
```

Expected: no output and exit code `0`.

## Same-Tick Save Oracles

The game must be paused on the loaded 1943 Ireland save and must not advance a
tick between save-oracle generation and the live probes.

Country economy oracle:

```powershell
python extractor\native\lanes\country_economy_counters_live\build_country_economy_oracle.py `
  --save "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4" `
  --out-dir "$run\country_economy_oracle"
```

Expected outputs:

- `$run\country_economy_oracle\economy.json`
- `$run\country_economy_oracle\economy_raw.json`
- `$run\country_economy_oracle\production.json`
- `$run\country_economy_oracle\oracle_status.json`

Success criteria: `oracle_status.json.status == "ok"` and save date is
`1943.11.9.17`. If the builder reports a binary-save/rakaly blocker, stop and
provide the uncompressed same-tick save or rakaly path; do not run oracle-backed
probes with stale output.

Independent derive verification:

```powershell
python extractor\native\lanes\country_economy_counters_live\verify_factory_totals_derive.py `
  --save "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4" `
  --economy-raw "$run\country_economy_oracle\economy_raw.json" `
  --out-json "$run\factory_totals_derive_verify.json"
```

Success criteria: owner-attributed independent recomputation reports `439/439`
matches for civilian, military, and dockyard `*_from_state_building_levels`.
Controller-attributed counts may differ and must be treated as occupation
context, not as the raw country-total oracle field. The state-plus-offmap total
row is only oracle self-consistency.

Market oracle:

```powershell
python extractor\save_oracle\parse_market.py `
  --out-dir "$run\market_oracle" `
  "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4"
```

Expected output: `$run\market_oracle\market.json`.

Success criteria: save date is `1943.11.9.17`. Use this only for same-frame
market stockpile comparisons.

## Offmap Factory Probe

Purpose: find the country offmap factory component after independent raw
state-building recomputation proved the owner-attributed state-backed component
is derivable.

```powershell
python extractor\memory_reader\economy\offmap_factory_probe_870ebf.py `
  --pid $hoi4Pid `
  --economy-raw "$run\country_economy_oracle\economy_raw.json" `
  --out-json "$run\offmap_factory_probe.json" `
  --sample-tags IRE ENG GER AST FRA EST NOR USA SOV JAP ITA CAN
```

Expected output: `$run\offmap_factory_probe.json`.

Hard failure criteria:

- `status == "build_gate_failed"`;
- `process.sha256` differs from the required SHA;
- selected countries do not include nonzero offmap examples and zero controls.

Candidate success criteria:

- candidate status is
  `CANDIDATE_ALL_SELECTED_EXACT_NEEDS_LIVE_DELTA` or
  `CANDIDATE_TRIPLE_ALL_SELECTED_EXACT_NEEDS_LIVE_DELTA`;
- candidate is not one of the known rejected paths
  `CCountry+0x1750`, `CCountry+0x17E0`, `CCountry+0x1038`, or
  `CCountry+0xF08`;
- the probe's `scan.excluded_ccountry_offsets` lists those rejected offsets, so
  they cannot appear as scalar or triple candidates in this run;
- value vector matches the same-save offmap oracle for nonzero and zero-control
  countries.

Promotion is still forbidden after this single snapshot. To validate a root,
run a before/after offmap fixture:

```powershell
python extractor\memory_reader\economy\offmap_factory_probe_870ebf.py `
  --pid $hoi4Pid `
  --economy-raw "$run\country_economy_oracle\economy_raw.json" `
  --out-json "$run\offmap_factory_before.json" `
  --sample-tags IRE ENG GER AST FRA EST NOR USA SOV JAP ITA CAN
```

Operator action: add or remove exactly one known offmap factory term for one
focus country, ideally IRE if the fixture can isolate it. Do not alter state
building levels, construction completion, occupation, or trade during the
fixture.

```powershell
python extractor\memory_reader\economy\offmap_factory_probe_870ebf.py `
  --pid $hoi4Pid `
  --economy-raw "$run\country_economy_oracle\economy_raw.json" `
  --out-json "$run\offmap_factory_after.json" `
  --sample-tags IRE ENG GER AST FRA EST NOR USA SOV JAP ITA CAN
```

Delta success criteria: the same candidate path changes by the exact known
offmap delta for the edited country and remains unchanged for control countries.
If no candidate moves exactly, offmap root remains open.

## Resource Flow Probe

Purpose: enumerate produced/imported/exported/used/deficit candidate containers
reachable from `CCountry` and `CProductionManager`.

Before running, create `$run\resource_ui_oracle.json` from values visible in the
same paused frame. Fill only values actually read from UI; omit unknown fields.
Read the resource names and numbers from the country resource topbar tooltip and
trade/resource UI for at least one importer, one exporter, and one control with
zero/low trade. Recommended sample tags: IRE, GER, ENG, SWE, TUR, USA.

Template:

```json
{
  "countries": {
    "IRE": {
      "resources": {
        "steel": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0},
        "oil": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0}
      }
    }
  }
}
```

Run:

```powershell
python extractor\memory_reader\economy\resource_flow_delta_probe_870ebf.py `
  --pid $hoi4Pid `
  --ui-oracle "$run\resource_ui_oracle.json" `
  --out-json "$run\resource_flow_probe.json" `
  --sample-tags IRE GER ENG SWE TUR USA SOV JAP ITA FRA `
  --min-countries 32 `
  --max-countries 96
```

Expected output: `$run\resource_flow_probe.json`.

Hard failure criteria:

- `status == "build_gate_failed"`;
- `process.sha256` differs from the required SHA;
- UI oracle was not collected from the same paused frame.

Candidate success criteria:

- path is not `CCountry+0x5B8` or `CCountry+0x1218` unless the new evidence
  directly answers the old rejection;
- `ui_oracle_matches.status == "CANDIDATE_UI_ORACLE_ALL_ELIGIBLE_MATCH"` for
  a named resource category and scale across at least two eligible countries;
- row resource ids are mapped to known resources, not only anonymous low ids.

Controlled delta fixture: change exactly one resource flow, for example one
trade import amount for IRE, then read the same UI fields and rerun the probe to
`$run\resource_flow_after.json`. A root is valid only if the same candidate path
moves by the known resource/category delta for the edited country and controls
remain stable. If only state deposits move, this is not a country flow root.

## Market Stockpile Probe

Purpose: find a new market stockpile/sellable equipment root without retrying
the rejected `CCountry+0x1230` path.

```powershell
python extractor\memory_reader\economy\market_license_rtti_probe_870ebf.py `
  --pid $hoi4Pid `
  --heap-scan `
  --market-oracle "$run\market_oracle\market.json" `
  --out-json "$run\market_license_rtti_market_stockpile.json" `
  --max-discovered-rtti 128 `
  --max-examples 96 `
  --max-objects 96
```

Expected output: `$run\market_license_rtti_market_stockpile.json`.

Hard failure criteria:

- `status == "build_gate_failed"`;
- `process.sha256` differs from the required SHA.

Candidate success criteria:

- current-build RTTI vtable exists for a market/stockpile/status class;
- heap bodies are observed;
- a country owner path exists;
- variant/amount row shape exists;
- same-frame market oracle can match rows by country tag and equipment variant.

Explicit rejection: `country_owner_paths.rejected_inline_path.path` equal to
`CCountry+0x1230` remains rejected no-retry unless a new non-stockpile semantics
claim is separately proven. Do not count `CCountry+0x1230` as a market stockpile
candidate from this run.

## Active License Fixture

Purpose: find a production-license detail root. Prior vectors
`CCountry+0x1070/+0x10A0` are rejected; relation context alone is not license
detail.

Operator fixture:

1. In the live game, create or accept one active production license involving a
   unique equipment type and two known countries.
2. Keep the game paused immediately after the license exists.
3. Record the expected pair and equipment in a note beside the output, for
   example `licensor=GER`, `licensee=IRE`, `equipment=<visible name>`.

Run:

```powershell
python extractor\memory_reader\economy\market_license_rtti_probe_870ebf.py `
  --pid $hoi4Pid `
  --heap-scan `
  --expect-active-license `
  --out-json "$run\market_license_rtti_active_license.json" `
  --max-discovered-rtti 128 `
  --max-examples 96 `
  --max-objects 128
```

Expected output: `$run\market_license_rtti_active_license.json`.

Hard failure criteria:

- `status == "build_gate_failed"`;
- `process.sha256` differs from the required SHA;
- the active license fixture was not actually present.

Candidate success criteria:

- `classification.license_fixture_status` indicates active license objects were
  observed;
- current-build RTTI is present for license detail/status classes, not only
  `CLicenceProductionAccessRelation`;
- a country owner path or nested object path links the object to the expected
  licensor/licensee/equipment fixture;
- prior `CCountry+0x1070/+0x10A0` dense consumer-vector rejection is not reused
  as positive evidence.

If only relation/access objects are observed, licensing remains open and owned
detail is not rooted.

## Prior Rejections To Preserve

- Factory total scalars: `CCountry+0x1750`, `CCountry+0x17E0`,
  `CCountry+0x1038`; rejected by v3/v4 same-save raw oracle evidence.
- `CCountry+0xF08`: real per-country data but unmapped to any raw/derived
  factory oracle field; delta-only candidate, not a known factory total.
- Market stockpile: `CCountry+0x1230`; rejected by same-tick fixture with
  272 live rows, 494 save rows, 0 exact matches.
- Resource flow: `CCountry+0x5B8` and `CCountry+0x1218`; rejected by market lane
  resource probes and state-shape proof.
- Licensing: `CCountry+0x1070/+0x10A0`; rejected as dense country-indexed
  vectors to one shared `CLicensedProductionConsumer`, not detail rows.

## End State

The expected end of the live session is a set of JSON proof files under `$run`.
Do not edit shared collector/recorder code from this runsheet. Do not claim any
root is validated unless the criteria above are met with the live JSON evidence.
