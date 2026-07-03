# Domain 58 Final Integration Packet

Lane: `country_economy_counters_live`  
Build: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`  
Evidence date: 2026-07-03  
Save/tick: Ireland run, `1943.11.9.17`  
Disposition: no domain-58 economy counter rows promoted; anchors validated and reusable.

## Evidence Artifacts

- `extractor/native/runs/country_economy_counters_live_870ebf_20260703_ireland_v1/country_economy_probe_baseline.json`
- `extractor/native/runs/country_economy_counters_live_870ebf_20260703_ireland_v1/country_economy_probe_with_oracle_v2.json`
- `extractor/native/runs/country_economy_counters_live_870ebf_20260703_ireland_v1/country_economy_probe_factory_field_validation_v3.json`
- `extractor/native/runs/country_economy_counters_live_870ebf_20260703_ireland_v1/country_economy_probe_factory_raw_validation_v4.json`
- `extractor/save_oracle/output/economy.json`
- `extractor/save_oracle/output/economy_raw.json`
- `extractor/save_oracle/output/production.json`

## Validated Anchors

These roots are validated and reusable by future domain-58 work:

| Anchor | Validated value |
| --- | --- |
| `CGameState` | `module+0x3316E90` |
| country table | `CGameState+0x310` |
| country count | `CGameState+0x31C = 440` |
| `CCountry` vtable | `module+0x27D0E90` |
| tag echo | `CCountry+0x8` |
| production manager | `CCountry+0xF68` |
| production line vector | `CProductionManager+0x58/0x60/0x64` |
| production line count in broad baseline/v2 sample | `394` |

The probe also records overlap anchors that are intentionally not emitted here:

- fuel status: `CCountry+0x1580`, owned by domain 10;
- production line base rows: owned by domain 11.

## Factory Totals And Consumer Goods

Final status: `ROOT_DISCOVERY_OPEN`, no offset promotion.

The live game did not expose civilian, military, or dockyard factory totals as stable per-country `CCountry` scalar fields in this build. Both static oracle cycles failed:

- v3 derived-oracle validation: `490` field rows, `0` validated.
- v4 raw-total validation: `506` field rows, `0` validated.

The v4 raw oracle deliberately removed free/used derived fields and compared only raw save fields or sums of raw stored rows:

| Raw field | Countries with data |
| --- | ---: |
| `factories_raw.civilian_total` | 439 |
| `factories_raw.military_total` | 439 |
| `factories_raw.dockyard_total` | 439 |
| `factories_raw.civilian_from_state_building_levels` | 439 |
| `factories_raw.military_from_state_building_levels` | 439 |
| `factories_raw.dockyard_from_state_building_levels` | 439 |
| `factories_raw.civilian_offmap_building_levels` | 439 |
| `factories_raw.military_offmap_building_levels` | 439 |
| `factories_raw.dockyard_offmap_building_levels` | 439 |
| `factories_raw.construction_line_count` | 439 |
| `factories_raw.construction_active_factories_sum` | 439 |
| `factories_raw.military_assigned_active_factories_sum` | 230 |
| `factories_raw.dockyard_assigned_active_factories_sum` | 230 |
| `consumer_goods_raw.*` | 0 |

No direct consumer-goods factory or factor scalar was found in the save country blocks; the raw oracle emits these as explicit nulls.

### Rejected Candidate Offsets

| Offset | Evidence | Disposition |
| ---: | --- | --- |
| `CCountry+0x1750` | v4 `factories_raw.civilian_total`: IRE `10 == 10`, but ENG `1199422256`, SOV `-1942455120`, FRA `1478545776` | rejected as pointer/garbage low dword, not a factory field |
| `CCountry+0x17E0` | v4 `factories_raw.civilian_total`: GER `99 == 99`, but IRE/ENG/USA/SOV/JAP/ITA read `0`, FRA reads `1478545776` | rejected as country-local coincidence or sparse/unrelated storage |
| `CCountry+0x1038` | v3/v4 dockyard candidates: IRE/GER/USA/SOV/ITA/FRA read near-constant `20`, JAP `9`, ENG `16`; only GER matches dockyard total `20` | rejected as capped/shared/non-country-total value |
| `CCountry+0xF08` | v3/v4 reads distinct per-country values: IRE `10`, ENG `14`, GER `27`, USA `22`, SOV `14`, JAP `23`, ITA `29`, FRA `13`; matches no derived or raw oracle field across countries | retained as unmapped candidate for delta fixture only |

The static-oracle ceiling is reached: the save exposes raw state/offmap building levels, but the live per-country total appears computed on demand from state data rather than held as a stable `CCountry` scalar.

## Recommended Orchestrator Decision

Do not wire any domain-58 factory counter rows yet. Choose one of these two approaches:

1. Derive factory totals in replay:
   - derive state-backed civilian/military/dockyard totals from domain-23 state building levels;
   - add a small future extraction for country offmap factory levels if replay cannot already derive them from save/or event-side data;
   - compute `country.economy.factories.*.total = state_building_total + offmap_total` in the integration layer.

2. Run a before/after delta fixture:
   - choose one country, preferably IRE;
   - change a known factory count by a known amount in-game without unrelated changes;
   - re-probe candidate scalar ranges before and after;
   - promote only an offset that moves by exactly the known delta and remains tag-local across at least one control country.

The delta fixture may still find nothing if the game recomputes totals on demand rather than caching them per country.

## Resources

Final status: `ROOT_DISCOVERY_OPEN`, no offset promotion.

v2 resource-vector validation found:

- `resource_flow_vector_candidates.oracle_matches.validated_cross_country_count = 0`;
- no all-sampled produced/imported/exported/used/deficit country resource vector.

A future resource pass needs either a stronger live UI oracle or a before/after resource delta that proves a consistent resource id/name mapping and amount scale across countries.

## Production Residuals

Final status: candidate/open, no new domain-58 rows.

The broad v2 probe saw `240` production-line residual oracle matches, but the section remained `CANDIDATES_ONLY_UNTIL_CONTROLLED_DELTA`. Known domain-11 base fields remain owned by domain 11; requested factories, priority, equipment variant pointer, efficiency, shortages, conversion, and history residuals still need controlled-delta proof before domain-58 emission.

## Exclusions And Overlaps

Excluded from domain 58:

- fuel: domain 10;
- stability and war support: domain 14;
- base production line active/output rows: domain 11;
- market contracts, market stockpile/sellable, licensing, and trade effects: domain 43;
- state resource deposits: state/map detail lanes.

Market/licensing candidates previously rejected or blocked by the market lane stay rejected; this lane adds no evidence to reopen them.

## Compact Schema Impact

No compact row layout should be registered for domain-58-owned factory/resource economy counters from this lane run.

If the orchestrator chooses the derive path, the replay schema fold should use explicit derived provenance:

```json
{
  "countries": {
    "<TAG>": {
      "economy": {
        "factories": {
          "civilian": {"total": null, "source": "derived_state_plus_offmap"},
          "military": {"total": null, "source": "derived_state_plus_offmap"},
          "dockyards": {"total": null, "source": "derived_state_plus_offmap"}
        }
      }
    }
  }
}
```

Three-state semantics for integration:

- `present`: validated live domain row or accepted derived value with provenance;
- `null`: required by replay schema but not validated/derived yet;
- `owned_elsewhere`: value must be joined from another domain.
