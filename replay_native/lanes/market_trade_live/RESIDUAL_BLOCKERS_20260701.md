# market_trade_live residual blockers - 2026-07-01

Domain: `43` (`market_trade_live`)
Lane: `trade_economy_full_residuals`
Status: `NOT COMPLETE` for full trade economy.

This file separates captured slices from exact blockers. Residual badges narrow
the backlog, but they do not complete this lane unless the data is production
captured, conclusively rejected, or fixture/tooling blocked with exact evidence.

## Captured Or Narrowed

### Purchase-contract core

Production capture exists in the shared compact stream as domain `43` v1 rows.
The current build roots are:

- `CPurchaseContract.vtable_rva = 0x2954C70`
- stale `16022851` `CPurchaseContract.vtable_rva = 0x2944B80` rejected for
  current `870ebf`
- `CEquipmentVariant.vtable_rva = 0x293AFB8`
- contract id: `+0x0C`
- seller tag db id: `+0x58`
- buyer tag db id: `+0x5C`
- equipment subbuffer: `+0x38`
- delivery subbuffer amount: `+0x80`
- buyer pointer: `+0x128`

Proof:

- `replay_native/runs/market_trade_live_dtag_shared_20260701T174028/collector_stream.bin`
- `market_trade_gate --min-game-hours 12`: 71 data records, 71 unique game
  hours, 3,555 emitted rows, 3,621 observed rows, 66 omitted candidates,
  0 partial rows, 0 skipped hours.
- `compact_replay_gate`: passed, no errors, no warnings, `.battleplan`
  verified.
- `pack_replay verify`: `verify: OK`, game
  `native_a420_18be1a7d7a198578`.

### Dynamic D-tag buyer identity

Shared patch `1f38b65d1` resolved the D-tag buyer path. The lane proof now
records it as a validated slice, not a remaining shared blocker.

Evidence:

- `market_trade_live_d04_identity_20260701_current`: `tag_table[368] == D04`,
  `country_table[368]` equals the purchase-contract buyer pointer, and the
  pointer has the current `CCountry` vtable.
- D-tag shared proof: 0 partial rows over the current run.

### Omitted-candidate classification beyond D-tag buyer rows

Proof:

- `replay_native/runs/market_trade_live_omitted_20260701_v1/omitted_candidate_probe.json`

Result:

- candidate rows: 51
- emitted complete rows: 50
- omitted-not-partial rows: 1
- missing-extractable rows: 0
- reason for omitted row: `candidate_vtable_invalid`
- stale `0x2944B80` vtable hits: 0
- valid dynamic fallback rows: 7 (`D01` through `D07`)

Backlog impact: current omitted candidates are classified as harmless
non-contract/stale candidates. Any candidate with unresolved country, variant,
delivery amount, or buyer pointer still fails as a structural partial.

### Price detail

Proof:

- `replay_native/runs/market_trade_live_detail_20260701_v1/purchase_contract_detail_probe.json`
- `replay_native/runs/market_trade_live_detail_20260701_v1/purchase_contract_detail_stream.bin`

Result:

- detail rows: 50
- price-valid rows: 50
- amount-valid rows: 50
- speed-valid rows: 0
- delivery-route-valid rows: 0
- progress-valid rows: 0
- duplicate conflicts: 0
- lane-local replay pack: `replay_pack/native_session.battleplan` verified with
  one `collector_compact_stream` entry and byte-equal stream readback.
- independent `pack_replay verify`: `verify: OK` for
  `market_trade_live_detail_v1`.

Validated source:

- price source:
  `CPurchaseContract+0x38 equipment_subbuf -> CEquipmentVariant payload i32 amount`
- amount source:
  `CPurchaseContract+0x80 delivery_subbuf -> CEquipmentVariant payload i32 amount`

Backlog impact: purchase-contract price is no longer schema-blocked. It is
represented by `MTD1` rows with anti-fabrication flags and Rust decoder gates.

### Purchase-contract field census

Proof:

- `replay_native/runs/market_trade_live_contract_fields_20260701_v1/purchase_contract_field_census.json`

Result:

- sampled/accepted current contract rows: 57/57
- current `CPurchaseContract.vtable_rva = 0x2954C70`
- current `CEquipmentVariant.vtable_rva = 0x293AFB8`
- plausible scalar offsets exist, including repeated small fields at `0x1C`,
  `0x28`, `0x2C`, `0x34`, `0x4C`, `0x64`, `0x70`, `0x74`, `0x7C`, and `0x94`
- pointer-like offsets exist, including `0x190`, `0x20`, `0x68`, `0xC8`,
  `0x178`, and `0x130`
- accepted for emission: false

Field-census blocker:

- Offsets are structural candidates only. No speed/completed/delivery/progress
  field is semantically accepted without a same-tick save/UI fixture keyed by
  `contract_id`.

Backlog impact: this narrowed the field search to current-build contract
objects but did not itself accept any detail field.

### Same-tick contract detail slice

Proof:

- `replay_native/runs/market_trade_live_contract_same_tick_20260701_v3/purchase_contract_same_tick_probe.json`
- `replay_native/runs/market_trade_live_contract_same_tick_20260701_v3/purchase_contract_same_tick_stream.bin`
- `replay_native/runs/market_trade_live_contract_same_tick_20260701_v3/replay_pack/native_session.battleplan`

Source evidence:

- `save_oracle/parse_market.py` extracts contract draft `speed`.
- `save_oracle/parse_market.py` extracts `contract_meta.completed`.
- `save_oracle/parse_market.py` extracts delivery country, assigned/total
  convoys, efficiency, days, `contract_delivery_state.factory_cic_progress`,
  and `contract_delivery_state.equipment_transfer_progress`.
- `replay/store.py` normalizes both into `diplomacy_world.trade_contracts` and
  per-country `diplomacy.lend_lease`.

Validated roots and fields:

- same-tick fixture: save `1943.11.9.17`, live `1943.11.9.17`, delta `0`
  game hours.
- joined contracts: 63.
- `CPurchaseContract+0xC0`: speed, 63/63 exact.
- `CPurchaseContract+0x25C`: completed, 63/63 exact.
- `CPurchaseContract+0x1D0`: days, 63/63 exact.
- `CPurchaseContract+0x5C`: delivery-country tag db id in this fixture,
  63/63 exact.
- `CPurchaseContract+0x118`: assigned convoys, 25/25 exact where the save
  exposes convoy counts.
- `CPurchaseContract+0x11C`: total convoys, 25/25 exact where the save exposes
  convoy counts.
- `CPurchaseContract+0x1C0`: efficiency raw, 63/63 exact.
- `CPurchaseContract+0x1E0`: factory CIC progress raw, 63/63 exact.
- `CPurchaseContract+0x1E8`: equipment-transfer progress raw, 63/63 exact.

Compact/replay proof:

- `MTD1` detail rows: 63.
- speed-valid rows: 63.
- completed-valid rows: 63.
- days-valid rows: 63.
- delivery-country-valid rows: 63.
- convoy-valid rows: 25.
- efficiency-valid rows: 63.
- factory-progress-valid rows: 63.
- equipment-progress-valid rows: 63.
- Rust gate: `ok=true`, one unique game hour, zero skipped hours, zero partial
  rows, no parse errors.
- `replay_pack_proof`: verified 7,200 compact stream bytes inside
  `.battleplan`.
- independent `pack_replay verify`: `verify: OK` for
  `market_trade_live_contract_same_tick_v3`.

Backlog impact: purchase-contract speed/completed are no longer a live-root
blocker. Delivery-country, days, convoys, efficiency, and factory/equipment
transfer progress are now a validated lane-local compact slice. Route/path
fields not represented by `MTD1` remain outside this slice until separately
rooted.

## Exact Remaining Blockers

### 1. Market stockpile and market automation

Source evidence:

- `save_oracle/parse_market.py` extracts
  `countries.<TAG>.equipment_market.market_stockpile.equipments`.
- `save_oracle/parse_market.py` extracts
  `market_request_automation` options.
- `replay/store.py` places those under country economy as `market_stockpile`
  and `market_automation`.

Current lane evidence:

- Domain 43 v1 emits live purchase contracts only.
- No `CCountry.equipment_market` live root for stockpile/automation is accepted.
- `market_stockpile_root_probe.py` v2 tested current-build RTTI names
  `CMarketStockpile`, `CEquipmentMarketStatus`, and
  `CMarketRequestAutomation`; none had RTTI string/COL/vtable hits in `870ebf`.
- The same probe scanned inline `CCountry` fields and found a strong but
  unaccepted `CCountry+0x1230` compact-vector candidate: stride `0x10`,
  `def_ptr + amount_raw`, type-70 variant-gated rows, 190 country tags with
  nonzero rows. This is not emitted because it is not yet proven to be sellable
  `market_stockpile` rather than another equipment bank.
- `stockpile_save_compare.py` v3 compared live `CCountry+0x1230` rows against
  the newest parsed `autosave.hoi4` market-oracle rows. It did not validate or
  reject the root because the save fixture was still stale: save `1943.9.1.1`,
  live `1943.9.18.23`, delta `430` game hours. The stale comparison produced
  270 live rows, 481 save rows, 0 exact matches, 24 amount mismatches,
  246 live-only rows, and 457 save-only rows, which cannot be used as
  same-tick rejection evidence.
- `stockpile_save_compare.py` v4 used a same-tick fixture from
  `IRE_1943_11_09_17.hoi4`: save date `1943.11.9.17`, live date
  `1943.11.9.17`, delta `0` game hours. This rejects `CCountry+0x1230` as
  the market-stockpile root: 272 live candidate rows, 494 save-oracle
  market-stockpile rows, 0 exact matches, 26 amount mismatches,
  246 live-only rows, and 468 save-only rows.

Rejected/narrowed path:

- Reject exact RTTI/vtable paths for
  `.?AVCMarketStockpile@NInternationalMarket@@`,
  `.?AVCEquipmentMarketStatus@NInternationalMarket@@`, and
  `.?AVCMarketRequestAutomation@NInternationalMarket@@` on current `870ebf`:
  0 string hits, 0 complete-object-locator hits, 0 vtable hits.
- Reject `CCountry+0x1230` as the market-stockpile root on current `870ebf`.

Blocker:

- No current-build live root/offset is accepted as the country sellable market
  stockpile or automation flags. The `CCountry+0x1230` path should not be
  retried without new code/fixture evidence showing it is a different
  replay-relevant equipment-market surface.

Required fixture:

- New market/equipment root evidence for country sellable equipment, plus at
  least one non-default automation flag.
- Acceptance requires compact `market_stockpile` rows keyed by country tag db
  id and equipment variant id. Automation still needs a separate accepted flag
  root.

### 2. Country resource import/export flows

Source evidence:

- `save_oracle/parse_economy.py` extracts
  `countries.<TAG>.resources.produced`.
- It also extracts `exported` and `to_export`.
- It derives `available = produced - exported`; there is no literal
  `available` key in the save.
- `replay/SCHEMA.md` reserves `resources.imported` and country-level
  import/export flow surfaces.

Current lane evidence:

- State resource deposits are separate map/state data.
- Domain 43 has no country resource-flow records.
- `resource_flow_root_probe.py` v1 tested the `CCountry+0x5B8` clue from
  `memory_reader/fuel_offsets.draft.json` and scanned nearby offsets
  `0x540..0x640` across 28 sampled countries.
- Result: 0 plausible resource-flow candidate layouts; no compact-vector row
  shape was accepted for production emission.
- `resource_flow_broad_probe.py` v1 scanned compact-vector shapes across
  `CCountry+0x400..0x1800` in 33 current countries. The only stable
  accepted-like family was `CCountry+0x1218`, stride `0x18`/`0x20`, with the
  uniform resource id sequence `[0]`.
- `resource_flow_state_shape_probe.py` v1 rejected that `CCountry+0x1218`
  family as the full produced/exported/to_export country-flow root. In the same
  live frame it validated 1,081 state roots, 718 state-resource rows, 467
  states with resources, 0 invalid resource vectors, and 0 invalid resource
  rows. All 28 compared countries had fewer candidate resource ids at
  `+0x1218` than their owned live state-resource id set; e.g. GER had candidate
  `[0]` versus six owned state-resource ids.

Rejected/narrowed path:

- Reject `CCountry+0x5B8` and nearby compact-vector interpretations as a
  production-ready country resource-flow root on current `870ebf`.
- Reject the stable `CCountry+0x1218` compact-vector family as the full
  per-country produced/exported/to_export flow root on current `870ebf`.

Blocker:

- No accepted current-build live country resource-flow root maps produced,
  exported, to_export, imported, available, or shortage by resource id. The
  `+0x5B8` and `+0x1218` paths should not be retried without new fixture/code
  evidence.

Required fixture:

- Same-tick country fixture with known resource produced/exported/to_export
  values, preferably with a shortage/importing country and an exporter.
- Acceptance requires compact `country_resource_flow` rows keyed by country and
  resource id, with explicit measured/derived flags.

### 3. License production detail

Source evidence:

- Diplomacy lane documents current-build
  `CLicenceProductionAccessRelation=0x2949858`.
- That relation flag proves access context only.

Current lane evidence:

- Domain 43 has no licensed-equipment, factory-cost, active license contract,
  or license production detail rows.
- `license_detail_root_probe.py` v1 found current-build RTTI/vtables for
  license-production candidates, including `CEquipmentProductionLicense`,
  `CLicensedProductionConsumer`, `CLicensedProductionStatus`, and the relation
  context `CLicenceProductionAccessRelation=0x2949858`.
- `license_detail_root_probe.py` v2 added a bounded `CCountry+0x400..0x2400`
  owner scan. It found 0 direct owner hits, and 15 unaccepted vector hits to
  `CLicensedProductionConsumer` at `CCountry+0x1070` / `CCountry+0x10A0`,
  concentrated on dynamic tags and a single object. These are candidate owner
  paths only; no row fields or semantics are validated.
- `license_vector_candidate_probe.py` v1 rejects the exact
  `CCountry+0x1070/+0x10A0` vector path as a production license-detail root:
  both vectors are dense `size=cap=440` country-indexed tables; all 15 hits
  reference one shared `CLicensedProductionConsumer` object; that object points
  to country `D55` but has 0 nested license/status objects and 0 equipment
  variant pointers in the bounded `0x800` object scan.

Blocker:

- Relation context is available in domain 53, but license production detail is
  not rooted in market/economy live memory. Current RTTI candidates are known;
  the `+0x1070/+0x10A0` consumer-vector path is now rejected. The remaining
  proof must find a different owned root or provide an active-license fixture.

Required fixture:

- Same-tick fixture with an active license production relationship and known
  licensed equipment/factory-cost/detail values.
- Acceptance requires a market/economy-owned compact `license_production` row,
  joined to diplomacy relation context by tag pair.

### 4. Market access and diplomatic trade context

Source evidence:

- Diplomacy lane pins:
  - `CMarketAccessRightsRelation=0x2A0AF20`
  - `CEquipmentPurchaseRelation=0x2A0AE58`
  - `CLicenceProductionAccessRelation=0x2949858`

Current lane evidence:

- These are relation flags owned by domain 53.
- Domain 43 should join by tag pair instead of duplicating relation-root scans.

Blocker:

- Shared cross-domain join record/registration is not present in this lane.

Required integration:

- Orchestrator-owned join from domain 43 purchase/stockpile/license rows to
  domain 53 relation context by `(seller_tag_db_id, buyer_tag_db_id)`.

### 5. Full lend-lease market-contract mapping

Source evidence:

- `save_oracle/parse_market.py` treats the game 1.19 equipment market as the
  cross-country transfer mechanism.
- `replay/store.py` attaches each purchase contract to both countries as
  `diplomacy.lend_lease` and keeps the canonical world list at
  `diplomacy_world.trade_contracts`.

Current lane evidence:

- Purchase-contract seller/buyer/equipment/amount and price are captured.
- `MTLL` maps each emitted purchase contract into exactly two directed rows:
  seller-outgoing and buyer-incoming, keyed by contract id, perspective tag,
  counterpart tag, equipment variant, amount, and optional price.
- `lend_lease_mapping_probe.py` v1 validated the current live slice: 63 core
  contracts, 126 mapping rows, 63 outgoing, 63 incoming, 126 amount-valid rows,
  126 price-valid rows, 0 asymmetric contracts, 0 duplicate keys, and
  0 partial/source/worker/bridge failures.
- The MTLL stream passed the Rust gate with one domain data record, one unique
  game hour, zero skipped hours, no parse errors, and a verified
  `.battleplan` container for `market_trade_live_lend_lease_mapping_v1`.
- Delivery detail and speed/completed fields are validated lane-local through
  `MTD1`, but not yet shared production. Market stockpile, non-market
  lend-lease roots, and relation context are not all captured through production
  compact records.

Blocker:

- The directed market-contract mapping backbone is validated. Full lend-lease
  replay is still blocked on shared production integration plus the remaining
  stockpile/resource/license/market-access context roots above.

Required acceptance:

- Canonical contract rows must include speed, prices, equipment, delivery
  country, convoys, delivery efficiency, days, delivery state, completed, and
  relation context joins.
- The per-country outgoing/incoming view must continue to come from the
  canonical MTLL rows without re-scanning or fabricating data.

## Current Proof Commands

```powershell
cargo test --manifest-path replay_native\lanes\market_trade_live\Cargo.toml
python replay_native\lanes\market_trade_live\tools\purchase_contract_field_census.py --pid 45880 --out-json replay_native\runs\market_trade_live_contract_fields_20260701_v1\purchase_contract_field_census.json --max-contracts 96
python save_oracle\parse_market.py --out-dir replay_native\runs\market_trade_live_save_oracle_20260701_v4 "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4"
python replay_native\lanes\market_trade_live\tools\stockpile_save_compare.py --pid 45880 --market-json replay_native\runs\market_trade_live_save_oracle_20260701_v4\market.json --out-json replay_native\runs\market_trade_live_stockpile_fixture_20260701_v4\stockpile_save_compare.json
python replay_native\lanes\market_trade_live\tools\resource_flow_broad_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_resource_flow_broad_20260701_v1\resource_flow_broad_probe.json --max-countries 96 --min-countries 32
python replay_native\lanes\market_trade_live\tools\resource_flow_state_shape_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_resource_flow_state_shape_20260701_v1\resource_flow_state_shape_probe.json --max-countries 96 --min-countries 32
python replay_native\lanes\market_trade_live\tools\license_detail_root_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_license_detail_20260701_v2\license_detail_root_probe.json --max-rtti 32 --max-strings 64 --max-examples 24
python replay_native\lanes\market_trade_live\tools\license_vector_candidate_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_license_vector_20260701_v1\license_vector_candidate_probe.json --max-examples 96 --max-objects 16
python replay_native\lanes\market_trade_live\tools\lend_lease_mapping_probe.py --out-bin replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --out-json replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_probe.json --examples 16
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\market_trade_live_dtag_shared_20260701T174028\collector_stream.bin --min-game-hours 12
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\market_trade_live_detail_20260701_v1\purchase_contract_detail_stream.bin --min-game-hours 1
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --min-game-hours 1
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\market_trade_live_residuals_20260701_v10\residual_badges_stream.bin --min-game-hours 1
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin replay_pack_proof -- --stream replay_native\runs\market_trade_live_detail_20260701_v1\purchase_contract_detail_stream.bin --out-dir replay_native\runs\market_trade_live_detail_20260701_v1\replay_pack --game-id market_trade_live_detail_v1 --min-game-hours 1
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin replay_pack_proof -- --stream replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --out-dir replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\replay_pack --game-id market_trade_live_lend_lease_mapping_v1 --min-game-hours 1
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin replay_pack_proof -- --stream replay_native\runs\market_trade_live_residuals_20260701_v10\residual_badges_stream.bin --out-dir replay_native\runs\market_trade_live_residuals_20260701_v10\replay_pack --game-id market_trade_live_residuals_v10 --min-game-hours 1
cargo run --manifest-path replay_native\lanes\compact_replay_verification_gates\Cargo.toml --bin compact_replay_gate -- --run-dir replay_native\runs\market_trade_live_dtag_shared_20260701T174028 --require-domain market_trade_live --min-game-hours 12 --write-report replay_native\runs\market_trade_live_dtag_shared_20260701T174028\compact_gate_report.json
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\market_trade_live_dtag_shared_20260701T174028\native_session.battleplan
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\market_trade_live_detail_20260701_v1\replay_pack\native_session.battleplan
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\replay_pack\native_session.battleplan
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\market_trade_live_residuals_20260701_v10\replay_pack\native_session.battleplan
```
