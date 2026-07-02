# market_trade_live registration patch plan - 2026-07-01

Domain: `43` (`market_trade_live`)
Lane: `trade_economy_full_residuals`
Shared-entrypoint owner: orchestrator

This lane packet does not edit `extractor/native/collector/src/lib.rs`. The
orchestrator should serialize the shared registration after reviewing the
lane-local decoder, proof streams, and blockers.

## Keep Existing Production Path

1. Keep one injected DLL collector/coordinator.
2. Keep domain id `43` and domain name `market_trade_live`.
3. Keep current v1 purchase-contract core rows compatible:
   - header length `48`
   - row length `48`
   - scale `100000`
   - row key `contract_id`
4. Keep shared D-tag country identity fallback from commit `1f38b65d1`.
5. Keep gate failure on:
   - `partial_field_rows > 0`
   - skipped game hours
   - source unavailable
   - worker panic
   - bridge unavailable
   - payload parse errors

## Register Detail Schema

Choose one orchestrator-managed path:

- Option A: append the lane-local `MTD1` trailer after v1 rows and bump only
  the domain-43 payload schema contract.
- Option B: register a sibling compact record kind for
  `purchase_contract_detail`.

Required record fields:

- `contract_id`
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

Required flags:

- `DETAIL_FLAG_PRICE_VALID`
- `DETAIL_FLAG_AMOUNT_VALID`
- `DETAIL_FLAG_SPEED_VALID`
- `DETAIL_FLAG_COMPLETED_VALID`
- `DETAIL_FLAG_DAYS_VALID`
- `DETAIL_FLAG_DELIVERY_COUNTRY_VALID`
- `DETAIL_FLAG_CONVOYS_VALID`
- `DETAIL_FLAG_EFFICIENCY_VALID`
- `DETAIL_FLAG_FACTORY_PROGRESS_VALID`
- `DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID`

Required decoder invariant:

- Nonzero unflagged optional fields are parse errors.
- Duplicate nonzero `contract_id` rows are parse errors.
- Price and amount may ship now because live proof validates them.
- Speed, completed, days, delivery-country tag id, convoy, efficiency, and
  factory/equipment-transfer progress fields may ship from the same-tick v3
  proof.

## Register Lend-Lease Mapping Schema

Choose one orchestrator-managed path:

- Option A: append the lane-local `MTLL` trailer after v1 rows and `MTD1`.
- Option B: register a sibling compact record kind for
  `lend_lease_market_contract_mapping`.

Required record fields:

- `contract_id`
- `perspective_tag_db_id`
- `counterpart_tag_db_id`
- `direction`
- `equipment_variant_id`
- `flags`
- `amount_raw`
- `price_raw`
- `source_record_kind`

Required flags and enums:

- `direction = outgoing`
- `direction = incoming`
- `LEND_LEASE_FLAG_AMOUNT_VALID`
- `LEND_LEASE_FLAG_PRICE_VALID`
- `source_record_kind = purchase_contract`

Required decoder invariant:

- Each contract id must have exactly one outgoing and one incoming row.
- Duplicate `(contract_id, perspective_tag_db_id, direction)` keys are parse
  errors.
- Amount and price must be zero unless their valid flag is set.
- `perspective_tag_db_id` and `counterpart_tag_db_id` must be valid, distinct
  tag database ids.
- `source_record_kind` must remain `purchase_contract` until a separate
  non-market lend-lease root is proven.

Current proof:

- `lend_lease_mapping_probe.py` v1 emitted 126 MTLL rows from 63 live purchase
  contracts: 63 outgoing, 63 incoming, 126 amount-valid, 126 price-valid,
  0 asymmetric contracts, 0 duplicate keys, and 0
  partial/source/worker/bridge failures.
- Rust gate accepted the stream with one unique game hour and zero skipped
  hours.
- `replay_pack_proof` and independent `pack_replay verify` passed for
  `market_trade_live_lend_lease_mapping_v1`.

## Register Residual Classification

Choose one orchestrator-managed path:

- Option A: append the lane-local `MTRB` trailer.
- Option B: register a sibling residual/classification record kind.

Required gate counters:

- `residual_badge_rows_total`
- `residual_source_blocked_rows_total`
- `residual_schema_blocked_rows_total`
- `residual_owned_elsewhere_rows_total`
- `residual_shared_patch_blocked_rows_total`
- `lend_lease_mapping_rows_total`
- `lend_lease_outgoing_rows_total`
- `lend_lease_incoming_rows_total`
- `lend_lease_amount_valid_rows_total`
- `lend_lease_price_valid_rows_total`

Current expected counters:

- rows: `10`
- source-blocked: `3`
- schema-blocked: `0`
- owned-elsewhere context: `1`
- shared-patch-blocked: `0`

## Future Record Kinds

Register only after the required fixture/root proof exists:

- `purchase_contract_delivery`
  - delivery country, convoys assigned/total, efficiency, days,
    factory progress, equipment transfer progress, completed.
  - accepted current fields from
    `purchase_contract_same_tick_probe.py` v3: `CPurchaseContract+0xC0` speed,
    `+0x25C` completed, `+0x1D0` days, `+0x5C` delivery-country tag db id,
    `+0x118/+0x11C` assigned/total convoys, `+0x1C0` efficiency raw,
    `+0x1E0` factory CIC progress raw, and `+0x1E8`
    equipment-transfer progress raw.
  - route/path fields not represented by `MTD1` remain unregistered until a
    separate fixture or code-root proof maps them to `contract_id`.
- `market_stockpile`
  - country tag db id, equipment variant id, amount, price-level/subsidy terms
    if assigned to this domain, and automation flags.
  - rejected candidate: `CCountry+0x1230` type-70 variant/amount vector. It
    was found for 190 country tags by `market_stockpile_root_probe.py` v2, then
    rejected by same-tick `stockpile_save_compare.py` v4 against
    `IRE_1943_11_09_17.hoi4`: 272 live candidate rows, 494 save-oracle
    market-stockpile rows, 0 exact matches, 26 amount mismatches,
    246 live-only rows, and 468 save-only rows. Do not register or retry this
    path as market stockpile without new code/fixture evidence.
  - fixture attempt: `stockpile_save_compare.py` v1 compared live
    `CCountry+0x1230` to parsed `autosave.hoi4`, but the save was 447 game
    hours behind live (`1943.6.1.1` vs `1943.6.19.16`), so the candidate is
    fixture-blocked, not validated or rejected.
  - stale fixture attempt: `stockpile_save_compare.py` v3 compared live
    `CCountry+0x1230` to parsed `autosave.hoi4`, but the save was still 430
    game hours behind live (`1943.9.1.1` vs `1943.9.18.23`), so the candidate
    remained fixture-blocked at that point.
  - remaining requirement: find a different live country equipment-market root
    for sellable stockpile rows and a separate automation flag root.
- `country_resource_flow`
  - country tag db id, resource id, produced/exported/to_export/imported,
    available/shortage, with measured/derived flags.
  - rejected candidate: `CCountry+0x5B8` and nearby `0x540..0x640`
    compact-vector interpretations from `resource_flow_root_probe.py` v1. Do
    not retry this path without new fixture/code evidence.
  - rejected candidate: stable `CCountry+0x1218` compact-vector family from
    `resource_flow_broad_probe.py` v1, rejected by
    `resource_flow_state_shape_probe.py` v1 as the full
    produced/exported/to_export flow root. Same-frame live state resources
    showed 718 state-resource rows, 0 invalid rows/vectors, and 28/28 compared
    countries with richer owned-state resource-id sets than the `[0]` candidate.
    Do not retry this path as a full flow root without new fixture/code
    evidence.
- `license_production`
  - licensed equipment, factory/cic cost, provider, licensee, active contract
    fields, joined to diplomacy relation context.
  - current candidates from `license_detail_root_probe.py` v1:
    `CEquipmentProductionLicense`, `CLicensedProductionConsumer`,
    `CLicensedProductionStatus`, and relation context
    `CLicenceProductionAccessRelation=0x2949858`.
  - bounded owner scan in `license_detail_root_probe.py` v2 found 0 direct
    `CCountry` owner hits and 15 unaccepted `CLicensedProductionConsumer`
    vector hits at `CCountry+0x1070/+0x10A0`.
  - `license_vector_candidate_probe.py` v1 rejects the exact
    `CCountry+0x1070/+0x10A0` path as a license-detail root: dense
    `size=cap=440` country-indexed vectors, one shared
    `CLicensedProductionConsumer`, and no nested license/status/equipment
    children. Do not register compact license detail rows until another
    market/economy-owned live container/root and row-field semantics are proven.
- `diplomatic_trade_context_join`
  - tag-pair join from domain 43 rows to domain 53 relation flags.

## Required Merge Gates

The orchestrator registration should not claim full trade economy complete until
all of these are true:

1. DLL-only capture emits the registered domain-43 and sibling rows.
2. Rust recorder decodes all registered rows.
3. `.battleplan` verification passes.
4. Hourly-or-better capture at speed five or current equivalent passes.
5. Zero skipped game hours.
6. No partial/source/worker/bridge failures.
7. Every remaining full trade-economy residual is either captured, conclusively
   rejected, or blocked by the exact fixture/tooling wall documented in
   `RESIDUAL_BLOCKERS_20260701.md`.

## Lane-Local Replay Proof

The lane now includes `replay_pack_proof`, a Rust proof binary that:

1. gates an existing domain-43 compact stream;
2. writes a minimal native-session manifest;
3. packs the stream and manifest into `native_session.battleplan`;
4. reopens the container through `snapshot::Container`;
5. runs `verify_all`;
6. reads the `collector_compact_stream` entry and byte-compares it to the input.

Current proof outputs:

- `extractor/native/runs/market_trade_live_detail_20260701_v1/replay_pack/`
- `extractor/native/runs/market_trade_live_residuals_20260701_v8/replay_pack/`
