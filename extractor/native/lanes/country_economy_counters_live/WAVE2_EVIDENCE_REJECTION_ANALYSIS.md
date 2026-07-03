# Wave-2 Evidence And Rejection Analysis

Scope: economy residual reverse-engineering for factory totals, resources,
market stockpile, and licensing on build
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Files Read

- `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
- `extractor/native/lanes/country_economy_counters_live/README.md`
- `extractor/native/lanes/country_economy_counters_live/INTEGRATION_PACKET.md`
- `extractor/native/lanes/country_economy_counters_live/build_country_economy_oracle.py`
- `extractor/native/lanes/country_economy_counters_live/probe_870ebf_country_economy_roots.py`
- `extractor/native/lanes/market_trade_live/README.md`
- `extractor/documents/archive/native_lanes/market_trade_live/RESIDUAL_BLOCKERS_20260701.md`
- `extractor/native/lanes/country_economy_production_stockpile_live/README.md`
- Probe models from `naval_missions_status_live`,
  `diplomacy_intel_operatives_live`, and
  `naval_combat_live/tools/rtti_vtable_rediscover.py`

## Factory Totals

Validated reusable anchors from d58:

- `CGameState@module+0x3316E90`
- country table `CGameState+0x310` / count `CGameState+0x31C`
- `CCountry` vtable `module+0x27D0E90`
- `CCountry+0x8` tag echo
- `CCountry+0xF68 -> CProductionManager*`
- production line vector `CProductionManager+0x58/+0x60/+0x64`

Rejected direct total candidates:

| Candidate | Proof | Disposition |
| --- | --- | --- |
| `CCountry+0x1750` | Same-save v4 raw oracle: IRE matched civilian total but ENG/SOV/FRA read pointer-like garbage. | Rejected total scalar. |
| `CCountry+0x17E0` | Same-save v4 raw oracle: GER matched civilian total but most sampled countries read zero or garbage. | Rejected total scalar. |
| `CCountry+0x1038` | v3/v4 dockyard candidates near-constant/capped; only GER matched. | Rejected dockyard total. |
| `CCountry+0xF08` | Real per-country values but no raw or derived oracle field match across countries. | Retained only as unmapped delta candidate, not a factory total. |

Derived path verification:

- Built `wave2_ire_oracle/economy_raw.json` from the 1943 Ireland save.
- `verify_factory_totals_derive.py` independently reparses raw save state
  building blocks through `parse_save.py`, sums
  `industrial_complex`/`arms_factory`/`dockyard` levels, and compares the result
  to `economy_raw.json` `*_from_state_building_levels`. It does not reuse
  `parse_economy.py`'s state aggregation.
- Owner-attributed state-level recomputation matches the oracle from-state
  fields exactly:
  - civilian: 439/439;
  - military: 439/439;
  - dockyard: 439/439.
- Controller-attributed recomputation is a separate occupation/controller view
  and does not match the owner-attributed oracle in this save:
  - civilian: 399/439;
  - military: 398/439;
  - dockyard: 420/439.
- The previous state-plus-offmap 439/439 row is retained only as oracle
  self-consistency: it checks that the oracle's own total equals its own
  from-states plus offmap fields. It is not the independent derive proof.
  Self-consistency counts are civilian 439/439, military 439/439, and dockyard
  439/439; nonzero residual/offmap countries are 38, 18, and 3 respectively.

Next candidate: do not search for direct total scalars again. Search only for
country offmap factory levels with `offmap_factory_probe_870ebf.py`, then
require a before/after offmap delta.

## Production Stockpile Boundary

The production stockpile lane has a validated ordinary stockpile path:

- `CCountry+0xF68 -> CProductionManager*`
- `CProductionManager+0x220/+0x228/+0x22C` stockpile vector
- type-70 equipment variant pointer plus amount rows

This is ordinary country production stockpile, not market sellable stockpile.
Do not reuse it as market evidence without a same-frame market oracle.

Still source/schema blocked in that lane:

- requested factory counts;
- consumer goods;
- resource flows/shortages;
- deployment queues;
- MIO design/variant production joins.

## Resources

Rejected resource-flow candidates:

| Candidate | Proof | Disposition |
| --- | --- | --- |
| `CCountry+0x5B8` and nearby `0x540..0x640` | `resource_flow_root_probe` found no production-ready produced/exported/to_export mapping. | Rejected. |
| `CCountry+0x1218` | Broad scan found stable-looking `[0]` resource id family, but `resource_flow_state_shape_probe` showed all compared countries had more owned state resource ids than candidate ids. | Rejected as full country resource-flow root. |

Next candidate: scan wider `CCountry` and `CProductionManager` vector-like
containers with `resource_flow_delta_probe_870ebf.py`, but treat known rejected
offsets as no-retry unless new evidence directly answers the old failure. The
probe needs UI rows for produced/imported/exported/used/deficit and an isolated
trade/resource delta before promotion.

## Market Stockpile

Validated market-trade slice:

- `CPurchaseContract.vtable_rva = 0x2954C70`
- contract id/type, seller/buyer tag ids, buyer pointer, equipment subbuffer,
  delivery subbuffer, price/amount, speed/completed/delivery detail
- `MTLL` directed mapping slice validates outgoing/incoming purchase-contract
  rows

Rejected market stockpile candidate:

| Candidate | Proof | Disposition |
| --- | --- | --- |
| `CCountry+0x1230` | Same-tick stockpile fixture: live and save both `1943.11.9.17`; 272 live rows vs 494 save rows, 0 exact matches, 26 amount mismatches, 246 live-only, 468 save-only. | Rejected no-retry as market stockpile root. |

RTTI/vtable paths for the guessed stockpile/status/automation classes had no
accepted current-build root in the prior market probe. Next candidate is a
broader current-build RTTI/heap scan with
`market_license_rtti_probe_870ebf.py`, excluding `CCountry+0x1230` as positive
evidence.

## Licensing

Known relation context:

- `CLicenceProductionAccessRelation` exists as diplomacy relation context.
- Relation context is not license production detail.

Rejected licensing candidate:

| Candidate | Proof | Disposition |
| --- | --- | --- |
| `CCountry+0x1070/+0x10A0` | Dense `size=cap=440` country-indexed vectors pointing to one shared `CLicensedProductionConsumer`; no nested license/status objects and no equipment variant pointers. | Rejected as license-detail root. |

Next candidate: run `market_license_rtti_probe_870ebf.py --expect-active-license`
only after the operator creates an active production-license fixture. Promotion
requires a detail/status object path tied to the expected licensor, licensee,
and equipment, not just relation access.

## Open Risks

- The factory derive spec proves raw save totals, not exact UI usable factories
  under occupation, damage, consumer goods, or trade.
- Offmap factories are the remaining total residual; no live root is validated.
- Resource flow UI oracle collection is manual and must be same-frame. Guessed
  zeros are not evidence.
- RTTI heap presence alone is not a row root; owner path and semantic oracle are
  required.
