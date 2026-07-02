# supply_logistics_attrition_live residual fixture contracts

These contracts are the fail-closed boundary for domain 44 residuals. A field may
replace its mandatory badge row only after the lane captures the proof listed
here on the same current-build frame family.

## Exact Province Supply / Remaining

`cprovince_residual_probe.py` accepts a tooltip anchor file:

```json
{
  "anchors": [
    {
      "province_id": 3544,
      "remaining": 19.34,
      "hubs": 16.94,
      "state": 2.40,
      "sum": 19.34
    }
  ]
}
```

Required proof:

- 8 to 12 same-frame anchors captured from the supply map tooltip, including at
  least one zero/near-zero province, one hub province, one naval hub/port
  province, one high-supply capital/core province, and two adjacent provinces
  with different values.
- For every anchor, record `province_id`, displayed remaining supply, hub
  component, state component, total/sum if shown, game date/hour, exe SHA, and
  screenshot or OCR text source path.
- Run `cprovince_residual_probe.py --anchors <file>` on the same frame.
- A candidate is still not production-safe after a same-frame match. It must
  also pass a temporal/network differential: toggle or damage a supply network
  input, capture a second anchor set, and prove the same chain/slot tracks
  `remaining` while rejecting sibling `hubs`, `state`, and `sum` aliases.

Until this exists, exact province supply stays a badge; the accepted replay
value remains the division/province proxy plus CSupply input rows.

## Attrition Flag / Reason

Required proof:

- A controlled division with a visible attrition reason or save/UI reason source
  on the same frame. Acceptable causes include severe low supply, equipment
  attrition, weather/terrain attrition, or naval transfer attrition, but the
  cause must be visible and tied to a specific division id.
- At least one positive division and two negative controls in the same country
  and nearby province set.
- Probe evidence that a specific division-body field, table entry, or function
  output changes with the visible reason and does not alias current supply,
  supply deficit, fuel, movement, combat, or template id.

Until then, domain 44 emits division current/max/gain supply, computed deficit,
and fuel only; attrition flag/reason remains a runtime-computed badge.

## Hub Motorization

Required proof:

- A controlled supply hub/node whose motorization level is toggled through the
  game UI or save fixture, with before/after proof for the same node province
  and country.
- A current-build per-country/node-setting join from the pinned
  `CCountrySupplySystem` RTTI class pool. The stale 16022851 vtable pair and
  the bounded +0x100F0 translation must not be reused; both are rejected by
  `root_blocker_probe.py` on 870ebf. Current-build RTTI vtables are
  `module+0x298BBF8/+0x298BC48`.
- Row semantics must carry node province, country/tag id, and motorization level
  `0..2`, with disabled/owner cases explicitly represented or badged.

Until then, supply hub buildings are exact but hub motorization remains badged.

Current fixture state:

- `extractor/native/runs/supply_logistics_attrition_live_save_oracle_870ebf_20260701_v3/supply.json`
  contains `230` node motorization entries across `40` countries from autosave date
  `1943.11.1.1`.
- `extractor/native/runs/supply_logistics_attrition_live_ccountry_node_records_870ebf_20260701_v1/CCOUNTRY_SUPPLY_NODE_RECORD_PROBE.md`
  validates `CCountrySupplySystem+0x78 -> CCountry` and
  `CCountrySupplySystem+0x20` as `0x30`-stride country/node/tag records, but rejects
  record `+0x04` as the exact motorization level candidate (`21` matches, `96`
  mismatches).
- `extractor/native/runs/supply_logistics_attrition_live_hub_motorization_level_870ebf_20260701_v1/HUB_MOTORIZATION_LEVEL_PROBE.md`
  proves the current oracle cannot validate an exact level field: all `490`
  matched save-level pairs are level `2`, and the best live candidate
  `u8@record+0x2D` still mismatches `20/117` matched actor-level records.
- These are valid save-oracle anchors, but they do not satisfy production live
  extraction until the exact level field or function output is proven by a same-frame
  toggle differential with at least two known levels, ideally one level `1` and one
  level `3`.

## Convoy / Logistics Pressure

Required proof:

- A controlled overseas or foreign-homebase supply route with visible convoy or
  logistics pressure for a specific country/node/province pair.
- A current-build field path from the pinned `CCountrySupplySystem` RTTI class
  pool that maps pressure to the same save-oracle concepts: `disrupted_supply`,
  `foreign_homebase_nodes`, truck/train loss rings, or an equivalent UI
  pressure source.
- Negative controls for a country with convoys but no active pressure, and a
  land-only supply area with no convoy dependency.

Until then, existing country convoy/fuel/equipment domains are not enough for
this lane; convoy/logistics pressure remains badged.

Current fixture state:

- The same autosave oracle contains zero disrupted-supply countries and `27`
  foreign-homebase entries across `8` countries. Of those, `25` entries across
  `7` tags join to current live `CCountrySupplySystem+0x78 -> CCountry`; tag
  `D04` is present in the save oracle but not in the live country join.
- `extractor/native/runs/supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v1/FOREIGN_HOMEBASE_PRESSURE_RECORD_PROBE.md`
  validates the strongest current foreign-homebase pressure candidate:
  `CCountrySupplySystem+0x1E0` points to `0x70`-stride records with province
  `+0x00`, supply `i32/100000@+0x08`, penalty `i32/100000@+0x18`, and duplicate
  province `+0x50`. The stale save oracle produced `25/25` joined province
  matches, `20` exact raw supply+penalty matches, and `25` duplicate-province
  matches; competing field/stride layouts scored lower.
- `extractor/native/runs/supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v2/domain44_pressure_candidate_payload.bin`
  carries the same layout through lane-local compact row kind `5`, with `46`
  candidate pressure rows plus the five mandatory wall badges. The Rust gate and
  `.battleplan` proof are in
  `extractor/native/runs/supply_logistics_attrition_live_pressure_replay_pack_870ebf_20260701_v1/REPLAY_PACK_PROOF.md`.
  This is a candidate compact path only; it does not remove the convoy-pressure
  wall badge or satisfy production capture.
- These anchors and candidate rows narrow the blocker to same-frame semantic validation
  and to roots for convoy/port/disrupted pressure. The compact rows are lane-local
  candidate data, not production pressure capture.

## CSupplySystem Arrays

Current 870ebf state:

- `CSupplySystem` object: vtables `module+0x295D0A8` and `module+0x295D0D0`.
- Production root hardening: `CSupplySystem = u64[CGameState+0x3D8]`, proven by
  `extractor/native/runs/supply_logistics_attrition_live_csupply_root_870ebf_20260701_v1/CSUPPLY_ROOT_PATH_PROBE.md`.
  Repeated direct-root validation averaged `27.719us` over `500` reads; the old
  in-frame heap-wide vtable scan measured `24922.93ms` in the lane-local proof and
  about `6.8s` in the shared compact run.
- `+0x10`: emitted as filtered per-province low-i32/1e5 efficiency/fulfilment
  input rows. Raw out-of-range or packed entries are rejected per value.
- `+0x138`: not emitted; current frame has negative and `>100000` values under
  the bounded low-i32/1e5 guard.
- `+0x150`: not emitted; current frame has short-read and invalid-value failures
  under the same guard.

Any future expansion of `+0x138` or `+0x150` must include same-frame semantic
anchors before replacing the CSupply rejection badge.
