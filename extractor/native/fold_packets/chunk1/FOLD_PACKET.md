# Fold Packet Chunk 1

Scope: draft folds for decoded compact domains 52, 23, 44, and 43. These files are intentionally standalone `rustc --test` modules with tiny world-fold stubs so the serial worker can move the logic into `recorder_host/src/world_fold.rs` mechanically.

Shared fold rules followed:

- Do not fabricate fields. Decoded rows only fill fields they actually carry; schema fields without decoded evidence stay `null` or absent.
- `compact_status_is_failure()` behavior is fail closed: failed hours are marked unknown for the affected sidecar/snapshot fields, prior accumulator state is retained, and empty structures are not fabricated.
- Delta domains require a full baseline. A delta before baseline records `delta_before_baseline` and leaves affected fields unknown for the hour.
- Country ids are re-keyed through the same TAG resolver shape as `world_fold.rs`: `tag_db_id -> TAG` when known, numeric fallback plus provenance note when unresolved.
- Successful measured empty frames are distinct from failed/unknown frames. D43 and D44 can legitimately replace with empty vectors/maps on a successful zero-row frame; D52 and D23 failed/delta-before-baseline frames stay unknown instead.

## Domain 52: occupation_resistance_compliance

Draft module: `fold_d52.rs`

Shape source:

- `recorder_host/src/main.rs` domain 52 dispatches to `occupation52::domain52_compact::decode_payload`.
- `recorder_host/src/occupation52/domain52_compact.rs` is the real compact contract used by the wrapper.

Decoded shape:

- Header, 52 bytes: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_state_rows: u32`, `occupied_state_rows: u32`, `wall_badge_rows: u32`, `scale_raw: u32`, `law_token_rows: u32`, `garrison_template_rows: u32`, `reserved0: u32`, `reserved1: u32`.
- Row, 64 bytes: `state_id: u32`, `owner_tag_db_id: u32`, `controller_tag_db_id: u32`, `occupied_country_tag_db_id: u32`, `resistance_raw: i32`, `compliance_raw: i32`, `resistance_speed_raw: i32`, `compliance_speed_raw: i32`, `garrison_need_raw: i32`, `garrison_equilibrium_raw: i32`, `garrison_progress_raw: i32`, `occupation_law_token_id: u32`, `garrison_template_id: u32`, `flags: u32`, `reserved1: u32`, `reserved2: u32`.
- Row flags: owner, controller, resistance, compliance, occupied-state, occupied-tag-derived, occupation-law-token, garrison-template, resistance-speed-unavailable, garrison-signals-unavailable, occupation-law-unavailable, resistance-speed, compliance-speed, occupation-status-root, country-default-law-token.

Target schema home:

- Primary: REPLAY_SCHEMA section 4, `states[*].occupation` resistance/compliance and occupier/occupied country references.
- `states[*].occupation_law` stays `null`: the decoded row carries only a hashed/default token id, not the law string.

Fold semantics:

- Collector emits a full baseline, then delta rows (`COMPACT_STATUS_DELTA_DATA`) from `CompactOccupationResistanceComplianceTracker`.
- Full baseline clears D52-owned state occupation fields, then applies all rows.
- Delta rows patch only listed states, and only after a baseline was seen.
- Identity keying is `state_id`; country references (`owner_tag_db_id`, `controller_tag_db_id`, `occupied_country_tag_db_id`) use TAG re-keying.

Failure handling:

- Any failure-status frame marks occupation data unknown for the hour, preserves the prior accumulator, and adds a provenance note.
- Delta before baseline records a `delta_before_baseline` domain problem and marks occupation unknown for the hour.

R2 deltas:

- `_r2_occupation_resistance_compliance_live` under `states[*].occupation` preserves `occupation_law_token_id`, token encoding/source badges, `garrison_template_id`, garrison scalar values, and unavailable badges.
- Serial worker should keep this clearly marked as an R2 live sidecar until a token-to-law join exists.

Draft contract row wording:

`Occupation/resistance/compliance state rows are decoded and folded into REPLAY_SCHEMA section 4 states[*].occupation with TAG re-keying; law hash/garrison-template ids are retained under _r2_occupation_resistance_compliance_live pending token joins.`

Tests:

- Command: `rustc --test extractor/native/fold_packets/chunk1/fold_d52.rs -o extractor/native/fold_packets/chunk1/fold_d52_test.exe`
- Result: `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`

## Domain 23: map_detail

Draft module: `fold_d23.rs`

Shape source:

- `recorder_host/src/main.rs` decodes compact domain 23 inline in `decode_map_detail_payload`.

Decoded shape:

- Header, 48 bytes: `elapsed_wall_ms: u64`, `state_count: u32`, `changed_rows: u32`, `status_flags: u32`, `province_count: u32`, `observed_state_rows: u32`, `observed_province_rows: u32`, `resource_count: u32`, `state_building_count: u32`, `province_building_count: u32`, `reserved: u32`.
- State row, 28 bytes: `state_id: u32`, `flags: u32`, `resource_first: u32`, `resource_len: u32`, `building_first: u32`, `building_len: u32`, `reserved: u32`.
- Province row, 20 bytes: `province_id: u32`, `state_id: u32`, `flags: u32`, `building_first: u32`, `building_len: u32`.
- Entry rows, 12 bytes each: resources use `resource_id: u32`, `amount_raw: i32`, `reserved: u32`; buildings use `type_id: u32`, `level: u32`, `reserved: u32`.
- Resource/building names come from the collector ids; unknown ids are preserved as `tid_0x...` / `type_0x...`.

Target schema home:

- Primary: REPLAY_SCHEMA section 4, `states[*].resources`, `states[*].buildings`, and `states[*].province_buildings`.
- Secondary renderer mirror: section 8 map layers do not define resources/buildings, so `map.map_detail_live` is an R2 extra layer only.

Fold semantics:

- Collector emits a full baseline first, then delta rows through `CompactMapDetailTracker`.
- Full baseline clears D23-owned state/province map-detail fields and the R2 mirror before applying rows.
- Delta rows patch listed states/provinces only after a baseline was seen, then recompute state-level `province_buildings` totals from known province rows.
- Identity keying is stable `state_id` and `province_id`; there are no country references.

Failure handling:

- Failure-status frames mark map detail unknown for the hour, retain the prior accumulator, and do not emit empty resource/building maps.
- Delta before baseline records `delta_before_baseline` and leaves D23 fields unknown for the hour.

R2 deltas:

- `map.map_detail_live.resources_by_state`, `buildings_by_state`, and `province_buildings_by_province` mirror the same folded data for renderer use because section 8 has no canonical resource/building map layer.

Draft contract row wording:

`State resources and state/province building levels are decoded and folded into REPLAY_SCHEMA section 4 states[*].resources, states[*].buildings, and states[*].province_buildings; an R2 map.map_detail_live mirror is retained for renderer map layers.`

Tests:

- Command: `rustc --test extractor/native/fold_packets/chunk1/fold_d23.rs -o extractor/native/fold_packets/chunk1/fold_d23_test.exe`
- Result: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`

## Domain 44: supply_logistics_attrition

Draft module: `fold_d44.rs`

Shape source:

- `recorder_host/src/main.rs` decodes compact domain 44 inline in `decode_supply_logistics_attrition_payload`.

Decoded shape:

- Header, 48 bytes in current recorder host: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `scale_raw: u32`, `province_rows: u32`, `division_rows: u32`, `badge_rows: u32`, `observed_province_roots: u32`, `observed_division_roots: u32`, `csupply_input_rows: u32`.
- Row, 40 bytes: `row_kind: u32`, `entity_id: u32`, `join_id: u32`, `owner_tag_db_id: u32`, `flags: u32`, `value_a_raw: i32`, `value_b_raw: i32`, `value_c_raw: i32`, `value_d_raw: i32`, `aux_raw: i32`.
- Row kinds: 1 province logistics, 2 division supply, 3 computed badge, 4 CSupply province input.
- Flags include rail, supply_node, naval_supply_hub, infrastructure, live_building_vector, division_current_supply, division_max_supply, division_supply_gain, division_owner, exact_supply_wall, attrition_wall, proxy_value, division_supply_deficit, division_fuel, motorization_wall, convoy_pressure_wall, csupply_candidate_rejected, csupply_efficiency, and csupply_province_array.

Target schema home:

- Primary: REPLAY_SCHEMA section 13.2, `supply.by_province[*]` with province-level logistics-derived fields, division supply entries, and source/confidence.
- No decoded row carries exact province tooltip supply, so `current/capacity/gain` for province tooltip supply remain absent/null unless a row explicitly carries a safe proxy.

Fold semantics:

- Current collector emits the current sampled rows on every successful frame; no DELTA_DATA tracker is used.
- Successful D44 frames are full replacement for the D44 supply sidecar. A successful zero-row frame is a measured empty sidecar.
- Province identity is `province_id`. Division rows are nested under `join_id` province and retain raw `entity_id` only as per-hour row identity; serial integration must not promote raw division id to durable `div_uid` without a domain-20 generation/type join.
- `owner_tag_db_id` on division rows uses TAG re-keying.

Failure handling:

- Failure-status frames mark supply unknown for the hour, preserve previous D44 supply accumulator, and add provenance.

R2 deltas:

- `supply.r2_logistics_attrition.badges` preserves exact-supply-wall, attrition-wall, proxy, motorization, convoy-pressure, and CSupply blocker badges.
- `supply.by_province[*].logistics_buildings` and `csupply_input` are decoded logistics details beyond the narrow schema 13.2 wall; keep them visibly R2/provenance-backed if the production schema does not accept those leaves.

Draft contract row wording:

`Supply/logistics subset is decoded and folded into REPLAY_SCHEMA section 13.2 supply.by_province with TAG re-keyed division owners; exact tooltip supply remains open and blocker badges are retained under r2_logistics_attrition.`

Tests:

- Command: `rustc --test extractor/native/fold_packets/chunk1/fold_d44.rs -o extractor/native/fold_packets/chunk1/fold_d44_test.exe`
- Result: `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`

## Domain 43: market_trade

Draft module: `fold_d43.rs`

Shape source:

- `recorder_host/src/main.rs` decodes compact domain 43 inline in `decode_market_trade_payload`.

Decoded shape:

- Header, 48 bytes: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_contract_rows: u32`, `emitted_contract_rows: u32`, `buyer_ptr_valid_rows: u32`, `variant_valid_rows: u32`, `amount_valid_rows: u32`, `partial_field_rows: u32`, `scale_raw: u32`.
- Row, 48 bytes: `contract_id: u32`, `contract_type: u32`, `flags: u32`, `seller_tag_db_id: u32`, `buyer_tag_db_id: u32`, `equipment_variant_id: u32`, `amount_raw: i64`, `equipment_subbuf_status: u32`, `delivery_subbuf_status: u32`, `buyer_ptr_status: u32`, `reserved: u32`.
- Flags: buyer-ptr-valid, equipment-subbuf-valid, delivery-subbuf-valid, variant-valid, amount-valid.

Target schema home:

- Primary: REPLAY_SCHEMA section 7b, `diplomacy_world.trade_contracts`.
- Per-country view: `countries[TAG].diplomacy.lend_lease` receives incoming/outgoing contract projections because the decoded rows are bilateral purchase contracts.
- Country economy fields stay null/absent: decoded D43 rows do not carry market stockpile, automation, resource-flow, price, speed, delivery progress, or completed-state data.

Fold semantics:

- Current collector scans the live current contract list each frame and does not use DELTA_DATA.
- Successful D43 frames are full replacement for trade contracts and D43-owned per-country contract projections. A successful zero-row frame is a measured empty current-contract list.
- Identity keying is `contract_id:contract_type` to avoid collapsing any reused id across contract type changes. Seller and buyer use TAG re-keying.

Failure handling:

- Failure-status frames mark trade contracts unknown for the hour, preserve previous contracts/projections, and add provenance.

R2 deltas:

- `trade_contracts[*].r2_market_trade_live` preserves raw `contract_type`, compact flags, and subbuffer/buyer pointer statuses.
- Schema fields without decoded values (`speed`, `prices`, `delivery_country`, `convoys`, `delivery_efficiency`, `days`, `delivery_state`, `completed`) remain null/absent.

Draft contract row wording:

`Purchase-contract rows are decoded and folded into REPLAY_SCHEMA section 7b diplomacy_world.trade_contracts plus per-country diplomacy.lend_lease projections with TAG re-keying; country economy stockpile/resource-flow fields remain open because decoded rows do not carry them.`

Tests:

- Command: `rustc --test extractor/native/fold_packets/chunk1/fold_d43.rs -o extractor/native/fold_packets/chunk1/fold_d43_test.exe`
- Result: `test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`

## Serial Worker Re-verify List

- Add these domain ids to the shared folded-domain list only when the production `world_fold.rs` arms are added, so they stop appearing under `unfolded_domains_this_hour` only after real folding is live.
- Confirm the production unknown-hour representation for new state/map/supply/trade slices. The drafts use tiny booleans, but production should follow the current null-plus-provenance convention rather than emit fabricated empties.
- Re-check D44 header length against current `recorder_host/src/main.rs`; lane-local D44 files mention later 52-byte pressure-candidate variants, but current recorder host decodes 48 bytes.
- Join D44 division rows through the production division identity map before attaching to canonical divisions. Raw `entity_id` alone is not a durable `div_uid`.
- Decide whether D43 per-country projections belong under `diplomacy.lend_lease` or a more specific country-economy/trade sidecar. The world contract list is the canonical schema home; decoded rows do not satisfy country economy stockpile/resource-flow fields.
- Keep D52 occupation-law values in R2 until a verified token-to-law-name join exists.

## No-schema-home Items

No domain is rejected as wholly without a schema home. The following decoded leaves are R2-only because the current schema home is incomplete or the row value is an opaque token/status:

- D52 law token ids and garrison template/scalar badges: `_r2_occupation_resistance_compliance_live`.
- D23 map-layer mirror of resources/buildings: `map.map_detail_live`.
- D44 logistics buildings, CSupply input, and blocker badges: `supply.by_province[*].logistics_buildings`, `csupply_input`, and `supply.r2_logistics_attrition`.
- D43 compact flags/subbuffer statuses and missing trade-detail fields: `trade_contracts[*].r2_market_trade_live`; omitted canonical fields remain null/absent.
