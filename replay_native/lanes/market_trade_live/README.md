# market_trade_live

Lane-local decoder, gates, probes, and registration plan for compact domain `43`
(`market_trade_live`).

Status: `NOT COMPLETE` for full trade economy. The lane now has production
purchase-contract core capture, dynamic D-tag buyer resolution through shared
commit `1f38b65d1`, omitted-candidate classification proof, and a lane-local
`MTD1` purchase-contract detail trailer that captures price/amount. The remaining
full trade-economy residuals are exact source/fixture blockers, not accepted
omissions.

## Validated Build Roots

Current live build/profile: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Validated roots and offsets:

- `CPurchaseContract` RTTI decorated name:
  `.?AVCPurchaseContract@NInternationalMarket@@`
- `CPurchaseContract.vtable_rva = 0x2954C70`
- rejected stale `16022851` purchase-contract vtable RVA: `0x2944B80`
- `CEquipmentVariant.vtable_rva = 0x293AFB8`
- purchase contract fields:
  - `id_type = +0x08`, expected `81`
  - `id_id = +0x0C`
  - `equipment_subbuf = +0x38`
  - `seller_tag_db_id = +0x58`
  - `buyer_tag_db_id = +0x5C`
  - `delivery_subbuf = +0x80`
  - `buyer_ptr = +0x128`
- price source:
  `CPurchaseContract+0x38 equipment_subbuf -> CEquipmentVariant payload i32 amount`
- transfer amount source:
  `CPurchaseContract+0x80 delivery_subbuf -> CEquipmentVariant payload i32 amount`

The shared D-tag buyer validator patch is already committed as `1f38b65d1`. The
proof run `replay_native/runs/market_trade_live_dtag_shared_20260701T174028/`
decoded 71 domain-43 data records, 71 unique game hours, 3,555 emitted rows,
3,621 observed rows, 66 omitted candidates, zero partial rows, zero skipped
hours, and a verified `.battleplan` container.

## Compact Payload

Shared v1 payload:

- domain id: `43`
- compact stream record magic: `H4CR`
- record version: `1`
- record header length: `32`
- data payload header length: `48`
- row length: `48`
- fixed scale: `100000`

Payload header fields:

- `u64 elapsed_wall_ms`
- `u32 row_count`
- `u32 changed_rows`
- `u32 status_flags`
- `u32 observed_contract_rows`
- `u32 emitted_contract_rows`
- `u32 buyer_ptr_valid_rows`
- `u32 variant_valid_rows`
- `u32 amount_valid_rows`
- `u32 partial_field_rows`
- `u32 scale_raw`

Rows:

- `u32 contract_id`
- `u32 contract_type`
- `u32 flags`
- `u32 seller_tag_db_id`
- `u32 buyer_tag_db_id`
- `u32 equipment_variant_id`
- `i64 amount_raw`
- `u32 equipment_subbuf_status`
- `u32 delivery_subbuf_status`
- `u32 buyer_ptr_status`
- `u32 reserved`

The gate allows omitted candidates only through
`observed_contract_rows - emitted_contract_rows`. It fails on partial field rows,
partial/source/worker/bridge status, decode errors, skipped game hours, or
insufficient hourly coverage.

## Optional Detail Trailer

Lane-local `MTD1` trailer, appended after the v1 purchase-contract rows:

- trailer magic: `MTD1`
- schema version: `1`
- trailer header length: `32`
- row length: `64`

Rows are keyed by `contract_id` and carry:

- `flags`
- `price_raw`
- `amount_raw`
- `speed_raw`
- `completed_raw`
- `days`
- `delivery_country_tag_db_id`
- `convoys_assigned`
- `convoys_total`
- `efficiency_raw`
- `factory_cic_progress_raw`
- `equipment_transfer_progress_raw`

Anti-fabrication rule: every nonzero optional field must have its corresponding
valid flag set. The decoder rejects unflagged speed/completed/delivery/progress
values.

Current proof:
`replay_native/runs/market_trade_live_contract_same_tick_20260701_v3/`.

`purchase_contract_same_tick_probe.py` joined 63 live purchase contracts to the
same-tick `IRE_1943_11_09_17.hoi4` save oracle and emitted 63 detail rows:

- price-valid rows: 63
- amount-valid rows: 63
- speed-valid rows: 63
- completed-valid rows: 63
- days-valid rows: 63
- delivery-country-valid rows: 63
- convoy-valid rows: 25
- efficiency-valid rows: 63
- progress-valid rows: 63

The Rust gate accepted
`purchase_contract_same_tick_stream.bin` with one domain data record, one unique
game hour, zero skipped hours, zero partial rows, and zero parse errors.

The detail stream is also packed and verified as a lane-local replay container:
`replay_native/runs/market_trade_live_contract_same_tick_20260701_v3/replay_pack/`.
`replay_pack_proof.json` reports one `collector_compact_stream` entry, one
`recorder_manifest` entry, 7,200 verified stream bytes, and a byte-equal stream
readback from `native_session.battleplan`. Independent `pack_replay verify`
returns `verify: OK` for `market_trade_live_contract_same_tick_v3`.

Validated current-build direct offsets:

- `CPurchaseContract+0xC0`: speed, 63/63 exact.
- `CPurchaseContract+0x25C`: completed, 63/63 exact.
- `CPurchaseContract+0x1D0`: days, 63/63 exact.
- `CPurchaseContract+0x5C`: delivery-country tag db id, 63/63 exact.
- `CPurchaseContract+0x118/+0x11C`: assigned/total convoys, 25/25 exact.
- `CPurchaseContract+0x1C0`: efficiency raw, 63/63 exact.
- `CPurchaseContract+0x1E0`: factory CIC progress raw, 63/63 exact.
- `CPurchaseContract+0x1E8`: equipment-transfer progress raw, 63/63 exact.

## Optional Lend-Lease Mapping Trailer

Lane-local `MTLL` trailer, appended after the v1 purchase-contract rows:

- trailer magic: `MTLL`
- schema version: `1`
- trailer header length: `32`
- row length: `48`

Rows are keyed by `contract_id`, `perspective_tag_db_id`, and `direction`.
Every accepted purchase contract must produce exactly two rows:

- seller perspective: `direction = outgoing`
- buyer perspective: `direction = incoming`

Each row carries counterpart tag, equipment variant, amount, optional price, and
`source_record_kind = purchase_contract`. The decoder rejects duplicate
contract/perspective/direction keys and contracts without exactly one outgoing
and one incoming mapping row.

Current proof:
`replay_native/runs/market_trade_live_lend_lease_mapping_20260701_v1/`.

`lend_lease_mapping_probe.py` found 63 live purchase-contract rows and emitted
126 MTLL mapping rows:

- outgoing rows: 63
- incoming rows: 63
- amount-valid rows: 126
- price-valid rows: 126
- asymmetric contracts: 0
- duplicate mapping keys: 0
- partial/source/worker/bridge failures: 0

The Rust gate accepted `lend_lease_mapping_stream.bin` with one domain data
record, one unique game hour, zero skipped hours, 63 contract rows, and 126
decoded MTLL rows. The stream is packed and verified as
`market_trade_live_lend_lease_mapping_v1`; independent `pack_replay verify`
returns `verify: OK`.

This is a validated slice for the directed market-contract mapping backbone. It
does not complete full lend-lease replay because market stockpile/automation,
non-market lend-lease roots, and diplomacy relation joins are still separate
blockers.

## Residual Trailer

Lane-local `MTRB` residual trailer, appended after the v1 rows:

- trailer magic: `MTRB`
- schema version: `1`
- trailer header length: `32`
- badge row length: `48`

Current proof:
`replay_native/runs/market_trade_live_residuals_20260701_v12/`.

The refreshed residual proof classifies every assigned trade-economy residual:

- residual gap rows: 10
- validated slices: 6
- source-blocked residuals: 3
- schema-blocked residuals: 0
- owned-elsewhere context rows: 1
- shared-patch-blocked rows: 0
- Rust gate: passed, no parse errors

The residual stream is also packed and verified as a lane-local replay
container:
`replay_native/runs/market_trade_live_residuals_20260701_v12/replay_pack/`.
`replay_pack_proof.json` reports one `collector_compact_stream` entry, one
`recorder_manifest` entry, 624 verified stream bytes, and a byte-equal stream
readback from `native_session.battleplan`. Independent `pack_replay verify`
returns `verify: OK` for `market_trade_live_residuals_v12`.

Classified residuals:

- `purchase_contract_dynamic_tag_buyer_identity`: validated slice.
- `omitted_candidate_classification`: validated slice.
- `purchase_contract_delivery_route_progress`: validated slice.
- `purchase_contract_price_detail`: validated slice.
- `purchase_contract_speed_completed`: validated slice.
- `market_stockpile_sellable_equipment`: source-blocked.
- `country_resource_import_export_flows`: source-blocked.
- `license_production_contract_detail`: source-blocked.
- `market_access_diplomatic_context`: owned by diplomacy context.
- `lend_lease_market_contract_mapping`: validated slice for the core directed
  purchase-contract mapping, not full lend-lease replay.

Exact remaining blocker contract:
`replay_native/lanes/market_trade_live/RESIDUAL_BLOCKERS_20260701.md`.

Additional narrowed residual proofs:

- `replay_native/runs/market_trade_live_contract_fields_20260701_v1/purchase_contract_field_census.json`:
  sampled 57 current contract rows; rejected speed/completed/delivery/progress
  emission because candidate offsets are structural only without same-tick
  fixture validation keyed by `contract_id`.
- `replay_native/runs/market_trade_live_contract_same_tick_20260701_v2/purchase_contract_same_tick_probe.json`:
  same-tick `1943.11.9.17` fixture validates speed (`CPurchaseContract+0xC0`),
  completed (`+0x25C`), days (`+0x1D0`), delivery-country tag id (`+0x5C`),
  and convoys (`+0x118/+0x11C`) for compact `MTD1` emission. The same bounded
  direct/first-level pointer scan accepted no efficiency or progress root.
- `replay_native/runs/market_trade_live_contract_same_tick_20260701_v3/purchase_contract_same_tick_probe.json`:
  extends the same fixture to fixed-point raw values and validates efficiency
  (`CPurchaseContract+0x1C0`), factory CIC progress (`+0x1E0`), and
  equipment-transfer progress (`+0x1E8`) for compact `MTD1` emission.
- `replay_native/runs/market_trade_live_market_stockpile_20260701_v2/market_stockpile_root_probe.json`:
  no accepted RTTI/vtable root for the guessed market stockpile/status classes;
  found an unaccepted `CCountry+0x1230` type-70 variant/amount vector candidate
  for 190 countries that needs a same-tick market-stockpile fixture.
- `replay_native/runs/market_trade_live_stockpile_fixture_20260701_v1/stockpile_save_compare.json`:
  compared live `CCountry+0x1230` rows to parsed `autosave.hoi4` market
  stockpile rows. It is fixture-blocked, not rejected: save `1943.6.1.1`,
  live `1943.6.19.16`, delta `447` game hours.
- `replay_native/runs/market_trade_live_stockpile_fixture_20260701_v3/stockpile_save_compare.json`:
  repeated the comparison after a newer autosave. It is still fixture-blocked,
  not rejected: save `1943.9.1.1`, live `1943.9.18.23`, delta `430` game
  hours, 270 live rows, 481 save rows, and 0 exact matches.
- `replay_native/runs/market_trade_live_stockpile_fixture_20260701_v4/stockpile_save_compare.json`:
  rejects `CCountry+0x1230` as the market stockpile root with a same-tick
  fixture. Save and live are both `1943.11.9.17`; the candidate produced 272
  live rows while the save oracle had 494 market-stockpile rows, with
  0 exact matches, 26 amount mismatches, 246 live-only rows, and
  468 save-only rows.
- `replay_native/runs/market_trade_live_resource_flow_20260701_v1/resource_flow_root_probe.json`:
  rejected `CCountry+0x5B8` and nearby `0x540..0x640` compact-vector
  interpretations as a production-ready resource-flow root.
- `replay_native/runs/market_trade_live_resource_flow_broad_20260701_v1/resource_flow_broad_probe.json`:
  scanned compact-vector shapes across `CCountry+0x400..0x1800`. The only
  stable accepted-like family was `CCountry+0x1218` with resource id sequence
  `[0]`.
- `replay_native/runs/market_trade_live_resource_flow_state_shape_20260701_v1/resource_flow_state_shape_probe.json`:
  rejects `CCountry+0x1218` as the full produced/exported/to_export flow root.
  In the same live frame it read 1,081 state roots, 718 state-resource rows,
  467 states with resources, and 0 invalid rows/vectors; 28/28 compared
  countries had more owned-state resource ids than the `+0x1218` candidate.
- `replay_native/runs/market_trade_live_license_detail_20260701_v2/license_detail_root_probe.json`:
  found current-build license-production RTTI candidates, including
  `CEquipmentProductionLicense`, `CLicensedProductionConsumer`,
  `CLicensedProductionStatus`, and `CLicenceProductionAccessRelation=0x2949858`;
  found 0 direct `CCountry` owner hits and 15 unaccepted vector hits to
  `CLicensedProductionConsumer` at `CCountry+0x1070/+0x10A0`.
- `replay_native/runs/market_trade_live_license_vector_20260701_v1/license_vector_candidate_probe.json`:
  rejects the exact `CCountry+0x1070/+0x10A0` vector path as a license-detail
  root. The vectors are dense `size=cap=440` country-indexed tables; all hits
  reference one shared `CLicensedProductionConsumer` object, and that object has
  0 nested license/status objects and 0 equipment variant pointers.
- `replay_native/runs/market_trade_live_lend_lease_mapping_20260701_v1/lend_lease_mapping_probe.json`:
  validates the `MTLL` directed mapping slice from live purchase contracts: 63
  core contracts, 126 mapping rows, 63 outgoing, 63 incoming, 126 amount-valid,
  126 price-valid, 0 asymmetric contracts, 0 duplicate keys, and 0
  partial/source/worker/bridge failures.

## Omitted Candidate Proof

Current proof:
`replay_native/runs/market_trade_live_omitted_20260701_v1/omitted_candidate_probe.json`.

Result:

- current `CPurchaseContract.vtable_rva=0x2954C70`
- stale `0x2944B80` vtable rejected for `870ebf`
- candidate rows: 51
- emitted complete rows: 50
- omitted-not-partial rows: 1
- missing-extractable rows: 0
- omission reason: `candidate_vtable_invalid`
- strict country rows: 432
- valid dynamic fallback country rows: 7 (`D01` through `D07`)

So omitted candidates beyond D-tag buyer rows are classified as non-structural in
the current proof. Structural missing extractables still fail through
`partial_field_rows`.

## Local Commands

```powershell
cargo fmt --manifest-path replay_native\lanes\market_trade_live\Cargo.toml
python -m py_compile replay_native\lanes\market_trade_live\tools\purchase_contract_detail_probe.py replay_native\lanes\market_trade_live\tools\omitted_candidate_probe.py replay_native\lanes\market_trade_live\tools\market_stockpile_root_probe.py replay_native\lanes\market_trade_live\tools\resource_flow_root_probe.py replay_native\lanes\market_trade_live\tools\resource_flow_broad_probe.py replay_native\lanes\market_trade_live\tools\resource_flow_state_shape_probe.py replay_native\lanes\market_trade_live\tools\purchase_contract_field_census.py replay_native\lanes\market_trade_live\tools\license_detail_root_probe.py replay_native\lanes\market_trade_live\tools\license_vector_candidate_probe.py replay_native\lanes\market_trade_live\tools\stockpile_save_compare.py replay_native\lanes\market_trade_live\tools\lend_lease_mapping_probe.py
python replay_native\lanes\market_trade_live\tools\market_stockpile_root_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_market_stockpile_20260701_v2\market_stockpile_root_probe.json --max-examples 16
python replay_native\lanes\market_trade_live\tools\resource_flow_root_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_resource_flow_20260701_v1\resource_flow_root_probe.json --max-countries 64 --min-countries 24
python replay_native\lanes\market_trade_live\tools\resource_flow_broad_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_resource_flow_broad_20260701_v1\resource_flow_broad_probe.json --max-countries 96 --min-countries 32
python replay_native\lanes\market_trade_live\tools\resource_flow_state_shape_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_resource_flow_state_shape_20260701_v1\resource_flow_state_shape_probe.json --max-countries 96 --min-countries 32
python replay_native\lanes\market_trade_live\tools\purchase_contract_field_census.py --pid 45880 --out-json replay_native\runs\market_trade_live_contract_fields_20260701_v1\purchase_contract_field_census.json --max-contracts 96
python save_oracle\parse_market.py --out-dir replay_native\runs\market_trade_live_save_oracle_20260701_v4 "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4"
python replay_native\lanes\market_trade_live\tools\stockpile_save_compare.py --pid 45880 --market-json replay_native\runs\market_trade_live_save_oracle_20260701_v4\market.json --out-json replay_native\runs\market_trade_live_stockpile_fixture_20260701_v4\stockpile_save_compare.json
python replay_native\lanes\market_trade_live\tools\license_detail_root_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_license_detail_20260701_v2\license_detail_root_probe.json --max-rtti 32 --max-strings 64 --max-examples 24
python replay_native\lanes\market_trade_live\tools\license_vector_candidate_probe.py --pid 45880 --out-json replay_native\runs\market_trade_live_license_vector_20260701_v1\license_vector_candidate_probe.json --max-examples 96 --max-objects 16
python replay_native\lanes\market_trade_live\tools\lend_lease_mapping_probe.py --out-bin replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --out-json replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_probe.json --examples 16
cargo test --manifest-path replay_native\lanes\market_trade_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\market_trade_live_dtag_shared_20260701T174028\collector_stream.bin --min-game-hours 12
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\market_trade_live_detail_20260701_v1\purchase_contract_detail_stream.bin --min-game-hours 1
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin market_trade_gate -- --stream replay_native\runs\market_trade_live_lend_lease_mapping_20260701_v1\lend_lease_mapping_stream.bin --min-game-hours 1
cargo run --manifest-path replay_native\lanes\market_trade_live\Cargo.toml --bin residual_report -- --stream-out replay_native\runs\market_trade_live_residuals_20260701_v10\residual_badges_stream.bin --out replay_native\runs\market_trade_live_residuals_20260701_v10\residual_badges.json
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

## Registration Plan

Exact patch plan:
`replay_native/lanes/market_trade_live/REGISTRATION_PATCH_PLAN_20260701.md`.

The lane packet does not edit shared entrypoints. The orchestrator should
serialize shared registration after assigning final sibling record ids or a
domain-43 schema bump.
