# Chunk 2 World Fold Packet

Scope: draft fold logic for compact domains 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 42, and 46. All draft code in this directory is standalone and uses a tiny `WorldFold` stub surface matching the current `world_fold.rs` conventions: `compact_status_is_failure`, per-hour notes/problems, `delta_baselines_seen`, and numeric `tag_db_id` keys that the serial worker should re-key through the real `resolve_country_key`.

Current fold conventions observed:
- Failure status: if `COMPACT_STATUS_FAILURE_MASK` is set, mark the affected schema leaf unknown/null for the hour and add provenance/problem evidence. Do not fabricate empty collections.
- Delta domains: require a full baseline unless the collector/lane is documented as an implicit-baseline stream.
- Sparse update domains: several chunk2 collectors emit a full first sample and later sparse changed/deleted rows without setting `COMPACT_STATUS_DELTA_DATA`; the drafts treat "baseline already seen" as the sparse-update gate and still reject an explicit delta-before-baseline.
- Country references: keep `tag_db_id` in draft state and route through `resolve_country_key` on serial integration.
- Reused identity: include collector generations where emitted; do not key air wings/orders by bare ids.
- Extra live-only shape: when decoded rows exceed documented schema fields, use a clearly named `_r2_*` key and a provenance note instead of pretending the save-schema field exists.

## Serial Integration Outcome

Applied in `recorder_host/src/world_fold.rs` and `recorder_host/src/world_fold/tests.rs`.

Schema homes:
- D27: `order_of_battle.battle_plan_orders` as an R2-documented recorder projection pending schema-owner acceptance.
- D28: selected fields projected under `order_of_battle` command arrays and fleet/task-force hierarchy with `_r2_oob_command_capture`; raw `oob_command:*` rows remain outside canonical snapshots.
- D29: `order_of_battle.ships` and task-force ship lists as an R2-documented recorder projection pending schema-owner acceptance.
- D30/D32: D30 owns `order_of_battle.air_wings` existence; D32 attaches loadout/equipment annotations only to D30-owned wings. Loadout rows touched this hour, or stale rows invalidated by a D30 delete this hour, are pruned once at hour close if D30 proves the wing absent and are counted in current-hour provenance only.
- D31/D38/D42: `naval_air` and `strategic_air` R2 sidecars.
- D33/D35/D39/D46: `designs.equipment_variants_live`.
- D34: `strategic_air.air_bases`.
- D36/D37/D40: `designs._r2_air_variant_reserves`, `_r2_stockpile_variant_defs`, and `_r2_stockpile_variant_upgrades`.

Risk register outcomes:
- D29/D30/D31/D32/D34/D36/D38: collector tracker code emits a full first successful sample, then sparse changed/deleted rows without consistently setting `COMPACT_STATUS_DELTA_DATA`. The fold uses seen-once sparse gating for unflagged follow-up frames, preserves prior rows, and still rejects explicit delta-before-baseline.
- D27/D28: explicit delta domains. The generic delta-before-baseline guard is active.
- D33/D35/D37/D39/D40: catalog upsert domains. Missing rows mean unchanged; valid zero-row first success is a measured empty catalog for that domain leaf.
- D42: REPLAY_SCHEMA section 9 `events[]` is not recorder-owned. The fold keeps air combat losses under `_r2_air_combat_losses_live` only and does not inject `events[]`; replay-layer derived events remain separate.
- D46: collector always sets `COMPACT_STATUS_DELTA_DATA`, but the first successful frame is a complete implicit baseline. The exception is wired through `implicit_delta_baseline_or_sparse()` rather than bypassing the guard pattern.
- Low-bit deletion constants rechecked during integration: low bit `1` for D27/D28/D29/D30/D31/D32/D34/D36/D38/D46; D42 delete bit is `1 << 2`.
- Live decoder shape rechecked against `recorder_host/src/main.rs`: D29 header 40 with row-local compact strings; D30 header 36; D31 header 36; D32 header 44 with emitted equipment count at offset 28; D34/D35 header 36; D36/D42/D46 header 48; D38 header 52.

Post-integration cleanup: the standalone draft modules were applied and should be deleted after the final verification run; keep this packet note as the audit trail.

## D27 army_orders

Shape source: `recorder_host/src/main.rs::decode_compact_army_orders_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `root_rows: u32`, `observed_army_rows: u32`, `observed_order_rows: u32`, `target_state_count: u32`, `deleted_rows: u32`.
- Row: `army_id: u32`, `generation: u64`, `flags: u32`, `slot_index: u32`, `order_type: i32`, `target_first: u32`, `target_count: u32`, `target_state_declared_count: u32`, `target_status_code: u32`, `reserved0: u32`, `reserved1: u32`.
- Target row: `state_id: u32`, `reserved: u32`.

Target schema: `order_of_battle.battle_plan_orders` as an R2-documented recorder projection pending schema-owner acceptance.

Fold semantics: full replacement on non-delta first/full payload; `COMPACT_STATUS_DELTA_DATA` upsert/delete after baseline. Key `(army_id, generation, slot_index)`.

Failure/unknown handling: compact failure or delta-before-baseline makes `battle_plan_orders` null/unknown for that hour and records a domain problem. Valid empty payload means measured empty.

R2 delta: schema only has army/order/targets level fields. Preserve `generation`, `slot_index`, `target_state_declared_count`, and `target_status_code` under `_r2_army_order_capture` on each folded order.

Contract wording: `27 army_orders - PRODUCTION_SLICE_DONE; recorder_host decodes army order rows and target-state slices; folded into REPLAY_SCHEMA section 12 battle_plan_orders with failure-null and delta-baseline provenance.`

Draft module: `fold_d27.rs`.

## D28 oob_command

Shape source: `recorder_host/src/main.rs::decode_compact_oob_command_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `root_rows: u32`, `observed_army_rows: u32`, `observed_army_group_rows: u32`, `member_count: u32`, `deleted_rows: u32`, `observed_fleet_rows: u32`, `observed_task_force_rows: u32`.
- Row: `kind: u32`, `object_id: u32`, `flags: u32`, `parent_id: u32`, `owner_tag_db_id: u32`, `leader_id: u32`, `leader_skill: u32`, `member_first: u32`, `row_member_count: u32`, `logical_member_count: u32`, `member_count_engine: u32`, `missing_member_count: u32`, `leader_trait_count: u32`, `parent_status: u32`, `member_status: u32`, `owner_status: u32`, `leader_status: u32`, `leader_trait_status: u32`, `location_province_id: u32`, `reserved: u32`.
- Member row: `member_id: u32`, `reserved: u32`.

Target schema: selected fields projected under `REPLAY_SCHEMA.md` section 12 `order_of_battle` (`army_groups`, `armies`, `fleets`, task-force membership where available, `by_country`) with `_r2_oob_command_capture`; raw `oob_command:*` rows remain outside canonical snapshots.

Fold semantics: full replacement on non-delta first/full payload; `COMPACT_STATUS_DELTA_DATA` upsert/delete after baseline. Key `(kind, object_id)`.

Failure/unknown handling: compact failure or delta-before-baseline makes OOB command state null/unknown for that hour. Valid empty payload means measured empty.

R2 delta: preserve status codes, engine/logical member counts, missing-member count, leader trait count, and `location_province_id` under `_r2_oob_command_capture`. Re-key `owner_tag_db_id` through `resolve_country_key`.

Contract wording: `28 oob_command - PRODUCTION_SLICE_DONE; recorder_host decodes command tree rows and member slices; projected under REPLAY_SCHEMA section 12 order_of_battle with owner TAG re-keying, _r2_oob_command_capture evidence, failure-null behavior, and delta-baseline provenance; raw oob_command rows remain outside canonical snapshots pending schema-owner acceptance.`

Draft module: `fold_d28.rs`.

## D29 ships

Shape source: `recorder_host/src/main.rs::decode_compact_ships_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_ship_rows: u32`, `observed_task_force_rows: u32`, `deleted_rows: u32`, `scale_raw: u32`, `reserved: u32`.
- Row: `instance_id: u32`, `flags: u32`, `task_force_id: u32`, `category_enum: u32`, `strength_raw: u32`, `organisation_raw: u32`, `experience_raw: u32`, `sunk_convoys: u32`, `owner_tag_db_id: u32`, `reserved: u32`, `definition: compact string`.

Target schema: section 12 `order_of_battle.ships` live list and fleet/task-force linkage as an R2-documented recorder projection pending schema-owner acceptance. Section 13.1 `naval_air` does not receive direct ship rows.

Fold semantics: collector emits full first payload, then sparse changed/deleted rows but currently does not set `COMPACT_STATUS_DELTA_DATA`. Draft behavior: first successful payload clears/replaces; later successful payloads are sparse upsert/delete. Key `instance_id`. A resolved zero-current-ship frame is valid and carries tombstones; only source failure makes the leaf unknown and must not consume pending deletes.

Failure/unknown handling: compact failure makes `order_of_battle.ships` null/unknown for that hour and preserves accumulator for future successful hours. Explicit delta-before-baseline is rejected.

R2 delta: current schema comments say live `ships` does not carry owner. Preserve `owner_tag_db_id`/resolved owner under `_r2_ship_owner` until schema accepts owner on ship rows.

Contract wording: `29 ships - PRODUCTION_SLICE_DONE; recorder_host decodes live ship rows; projected under order_of_battle.ships with sparse-update preservation, owner TAG re-key provenance, failure-null behavior, and R2 evidence pending schema-owner acceptance.`

Draft module: `fold_d29.rs`.

## D30 air_wings

Shape source: `recorder_host/src/main.rs::decode_compact_air_wings_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_air_wing_rows: u32`, `deleted_rows: u32`, `scale_raw: u32`, `reserved: u32`.
- Row: `owner_tag_db_id: u32`, `wing_id: u32`, `generation: u64`, `flags: u32`, `count: u32`, `mission_mask: u32`, `mission_region: u32`, `effective_planes: u32`, `reserved: u32`.

Target schema: section 12 `order_of_battle.air_wings`; D30 owns air-wing existence.

Fold semantics: collector emits full first payload, then sparse changed/deleted rows without always setting `COMPACT_STATUS_DELTA_DATA`. Draft behavior: first successful payload clears/replaces; later successful payloads are sparse upsert/delete. Key `(owner_tag_db_id, wing_id, generation)`. A resolved zero-current-wing frame is valid and carries tombstones; only source failure makes `air_wings` unknown and retains the accumulator.

Failure/unknown handling: compact failure makes `air_wings` null/unknown for that hour and preserves accumulator. Explicit delta-before-baseline is rejected.

R2 delta: preserve `generation`, raw `mission_mask`, and `effective_planes` if the schema placement does not already expose them. Re-key owner through `resolve_country_key`.

Contract wording: `30 air_wings - PRODUCTION_SLICE_DONE; recorder_host decodes live air wing rows; folded into REPLAY_SCHEMA section 12 air_wings with generation-keyed sparse updates, owner TAG re-keying, and failure-null behavior.`

Draft module: `fold_d30.rs`.

## D31 air_region_activity

Shape source: `recorder_host/src/main.rs::decode_compact_air_region_activity_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_air_wing_rows: u32`, `observed_bucket_rows: u32`, `deleted_rows: u32`, `reserved: u32`.
- Row: `region_id: u32`, `owner_tag_db_id: u32`, `mission_mask: u32`, `flags: u32`, `wing_count: u32`, `plane_count: u32`, `effective_planes: u32`, `reserved: u32`.

Target schema: sections 13.1/14 as raw air-region activity input. The direct documented home is not a complete air-superiority result; use `naval_air._r2_air_region_activity` or `strategic_air._r2_air_region_activity`.

Fold semantics: full first payload, then sparse upsert/delete. Key `(region_id, owner_tag_db_id, mission_mask)`. A resolved zero-current-bucket frame is valid and carries tombstones; only source failure makes the activity leaf unknown and retains the accumulator.

Failure/unknown handling: compact failure makes the raw activity input leaf null/unknown for that hour. Explicit delta-before-baseline is rejected.

R2 delta: this is a raw live bucket, not a derived `air_superiority` total. Preserve it under `_r2_air_region_activity` with owner TAG re-keying.

Contract wording: `31 air_region_activity - PRODUCTION_SLICE_DONE; recorder_host decodes live air-region mission buckets; folded as REPLAY_SCHEMA sections 13.1/14 R2 air-region activity inputs with sparse-update preservation and failure-null behavior.`

Draft module: `fold_d31.rs`.

## D32 air_wing_loadouts

Shape source: `recorder_host/src/main.rs::decode_compact_air_wing_loadouts_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_air_wing_rows: u32`, `observed_equipment_rows: u32`, `emitted_equipment_rows: u32`, `deleted_rows: u32`, `scale_raw: u32`, `reserved: u32`.
- Row: `owner_tag_db_id: u32`, `wing_id: u32`, `generation: u64`, `flags: u32`, `air_base_id: u32`, `entry_first: u32`, `entry_count: u32`.
- Equipment row: `variant_id: u32`, `reserved: u32`, `amount_raw: i64`.

Target schema: section 12 `order_of_battle.air_wings` equipment/loadout annotation. D32 is attachment-only: orphan loadout rows without a D30-owned air-wing row are pruned with a current-hour provenance count and never create `air_wings[]` entries. A later D30 reappearance requires a fresh D32 loadout row to reattach loadout/equipment details.

Fold semantics: full first payload, then sparse upsert/delete. Key `(owner_tag_db_id, wing_id, generation)`. Equipment entries replace that wing/generation loadout. A resolved zero-current-loadout frame is valid and carries tombstones; only source failure makes loadouts unknown and retains the accumulator.

Failure/unknown handling: compact failure makes air wing loadouts null/unknown for that hour and preserves accumulator. Explicit delta-before-baseline is rejected.

R2 delta: preserve `air_base_id` and raw fixed-scale `amount_raw` under each air wing loadout if current schema lacks exact fields.

Contract wording: `32 air_wing_loadouts - PRODUCTION_SLICE_DONE; recorder_host decodes live air wing loadout rows and equipment slices; attaches section 12 air_wing loadout annotations only to D30-owned wings, drops orphan loadouts with provenance, and preserves generation-keyed sparse updates plus failure-null behavior.`

Draft module: `fold_d32.rs`.

## D33 air_variant_defs

Shape source: `recorder_host/src/main.rs::decode_compact_variant_defs_data_payload` for air variants.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_variant_rows: u32`, `observed_module_rows: u32`, `emitted_module_rows: u32`, `invalid_variant_rows: u32`, `reserved0: u32`, `reserved1: u32`.
- Row: `variant_id: u32`, `flags: u32`, `module_first: u32`, `module_count: u32`, `reserved0: u32`, `reserved1: u32`.
- Per-row strings after fixed rows: `display_name: compact string`, `variant_name: compact string`.
- Module row: `slot_handle: u32`, `reserved: u32`, `token: compact string`.

Target schema: section 11 `designs.equipment_variants_live[variant_id]` rather than save-keyed `equipment_variants[id:type]`.

Fold semantics: accumulating catalog upsert. Missing row means unchanged, not deleted. Key `variant_id`.

Failure/unknown handling: compact failure marks the live variant-definition leaf unknown for that hour but does not clear accumulated catalog state for later hours.

R2 delta: `variant_id` is a live bare id, not the save schema `id:type` key. Preserve under `equipment_variants_live` with a source/provenance badge.

Contract wording: `33 air_variant_defs - PRODUCTION_SLICE_DONE; recorder_host decodes live air variant definitions and module tokens; folded into REPLAY_SCHEMA section 11 equipment_variants_live with catalog-upsert semantics and failure-null behavior.`

Draft module: `fold_d33.rs`.

## D34 air_base_status

Shape source: `recorder_host/src/main.rs::decode_compact_air_base_status_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_air_base_rows: u32`, `observed_pool_rows: u32`, `deleted_rows: u32`, `reserved: u32`.
- Row: `air_base_id: u32`, `state_id: u32`, `flags: u32`, `capacity: u32`, `building_level: u32`, `based_wing_count: u32`, `based_plane_count: u32`, `reserved: u32`.

Target schema: section 14 `strategic_air.air_bases`.

Fold semantics: full first payload, then sparse upsert/delete. Key `air_base_id`. A resolved zero-current-air-base frame is valid and carries tombstones; only source failure makes `air_bases` unknown and retains the accumulator.

Failure/unknown handling: compact failure makes `strategic_air.air_bases` null/unknown for that hour and preserves accumulator. Explicit delta-before-baseline is rejected.

R2 delta: live fields are simpler than save-derived air-base schema. Preserve `building_level`, `based_wing_count`, and `based_plane_count` under `_r2_air_base_status` if exact schema fields are absent.

Contract wording: `34 air_base_status - PRODUCTION_SLICE_DONE; recorder_host decodes live air base capacity/status rows; folded into REPLAY_SCHEMA section 14 air_bases with sparse-update preservation and failure-null behavior.`

Draft module: `fold_d34.rs`.

## D35 air_variant_stats

Shape source: `recorder_host/src/main.rs::decode_compact_air_variant_stats_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_variant_rows: u32`, `invalid_variant_rows: u32`, `fixed_scale_raw: u32`, `range_scale_raw: u32`.
- Row: `variant_id: u32`, `flags: u32`, `equipment_token_raw: u32`, `equipment_type_token_raw: u32`, `interface_type_raw: u32`, `speed_cache_raw: i32`, `range_raw: i32`, `air_attack_raw: i32`, `agility_raw: i32`, `max_speed_raw: i32`, `reliability_raw: i32`, `air_defence_raw: i32`, `thrust_raw: i32`, `fuel_consumption_raw: i32`, `build_cost_ic_raw: i32`, `reserved: u32`.

Target schema: section 11. Current best home is `designs.equipment_variants_live[variant_id].live_stats`; do not force it into save-keyed historical stats unless the serial worker can join to `id:type`.

Fold semantics: accumulating catalog upsert. Missing row means unchanged. Key `variant_id`.

Failure/unknown handling: compact failure marks live stats unknown for the hour but preserves accumulated stats for later successful hours.

R2 delta: raw integer stat fields and token ids should live under `live_stats`/`_r2_air_variant_stats` with scale metadata (`fixed_scale_raw`, `range_scale_raw`).

Contract wording: `35 air_variant_stats - PRODUCTION_SLICE_DONE; recorder_host decodes live air variant stat rows; folded into REPLAY_SCHEMA section 11 equipment_variants_live live_stats with catalog-upsert semantics and failure-null behavior.`

Draft module: `fold_d35.rs`.

## D36 air_variant_reserves

Shape source: `recorder_host/src/main.rs::decode_compact_air_variant_reserves_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_country_rows: u32`, `observed_stockpile_rows: u32`, `observed_air_variant_rows: u32`, `matched_stockpile_rows: u32`, `invalid_country_rows: u32`, `deleted_rows: u32`, `scale_raw: u32`.
- Row: `tag_db_id: u32`, `variant_id: u32`, `flags: u32`, `reserved: u32`, `amount_raw: i64`.

Target schema: section 11 live designs/stockpile view. No exact save-schema stockpile field exists for bare live variant id; use `designs._r2_air_variant_reserves`.

Fold semantics: full first payload, then sparse upsert/delete. Key `(tag_db_id, variant_id)`. A resolved zero-current-reserve frame is valid and carries tombstones; missing country identity or unreadable stockpile source is failure and must not consume pending deletes.

Failure/unknown handling: compact failure makes the reserve leaf null/unknown for that hour. Explicit delta-before-baseline is rejected.

R2 delta: preserve `amount_raw` plus `scale_raw` and re-key `tag_db_id` to TAG.

Contract wording: `36 air_variant_reserves - PRODUCTION_SLICE_DONE; recorder_host decodes live air variant reserve rows; folded into REPLAY_SCHEMA section 11 R2 air variant reserves with TAG re-keying, sparse-update preservation, and failure-null behavior.`

Draft module: `fold_d36.rs`.

## D37 stockpile_variant_defs

Shape source: `recorder_host/src/main.rs::decode_compact_variant_defs_data_payload` for stockpile variants.

Decoded row shape: same wire shape as D33: header fields, variant rows, per-row `display_name`/`variant_name` strings, and module token rows.

Target schema: section 11 `designs.equipment_variants_live[variant_id]` or `designs._r2_stockpile_variant_defs` until the serial worker chooses a live stockpile variant leaf.

Fold semantics: accumulating catalog upsert. Missing row means unchanged, not deleted. Key `variant_id`.

Failure/unknown handling: compact failure marks stockpile variant definitions unknown for the hour but preserves accumulated catalog.

R2 delta: stockpile-side live variant ids may overlap air variant ids; keep a distinct `_r2_stockpile_variant_defs` namespace or source badge.

Contract wording: `37 stockpile_variant_defs - PRODUCTION_SLICE_DONE; recorder_host decodes live stockpile variant definitions and module tokens; folded into REPLAY_SCHEMA section 11 live variant/R2 stockpile definitions with catalog-upsert semantics and failure-null behavior.`

Draft module: `fold_d37.rs`.

## D38 air_superiority_inputs

Shape source: `recorder_host/src/main.rs::decode_compact_air_superiority_inputs_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_air_wing_rows: u32`, `observed_loadout_rows: u32`, `observed_equipment_rows: u32`, `observed_variant_rows: u32`, `invalid_air_wing_rows: u32`, `invalid_variant_rows: u32`, `deleted_rows: u32`, `scale_raw: u32`.
- Row: `region_id: u32`, `owner_tag_db_id: u32`, `mission_mask: u32`, `variant_id: u32`, `flags: u32`, `equipment_token_raw: u32`, `equipment_type_token_raw: u32`, `interface_type_raw: u32`, `wing_count: u32`, `reserved: u32`, `amount_raw: i64`.

Target schema: sections 13.1/14 raw air-superiority model input, best as `naval_air._r2_air_superiority_inputs` or `strategic_air._r2_air_superiority_inputs`.

Fold semantics: full first payload, then sparse upsert/delete. Key `(region_id, owner_tag_db_id, mission_mask, variant_id)`. A resolved zero-current-input frame is valid and carries tombstones; only source failure makes the model-input leaf unknown and retains the accumulator.

Failure/unknown handling: compact failure makes the raw model-input leaf null/unknown for the hour. Explicit delta-before-baseline is rejected.

R2 delta: not a final superiority score. Preserve raw `amount_raw`, token ids, `wing_count`, and scale metadata under `_r2_air_superiority_inputs`; re-key owner TAG.

Contract wording: `38 air_superiority_inputs - PRODUCTION_SLICE_DONE; recorder_host decodes live superiority model input rows; folded into REPLAY_SCHEMA sections 13.1/14 R2 air_superiority_inputs with sparse-update preservation and failure-null behavior.`

Draft module: `fold_d38.rs`.

## D39 air_variant_upgrades

Shape source: `recorder_host/src/main.rs::decode_compact_variant_upgrades_data_payload` for air variants.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_variant_rows: u32`, `observed_upgrade_rows: u32`, `emitted_upgrade_rows: u32`, `invalid_variant_rows: u32`, `reserved0: u32`, `reserved1: u32`.
- Row: `variant_id: u32`, `flags: u32`, `upgrade_first: u32`, `upgrade_count: u32`.
- Upgrade row: `upgrade_index: u32`, `level: u32`.

Target schema: section 11 `designs.equipment_variants_live[variant_id].upgrade_levels`.

Fold semantics: accumulating catalog upsert. The row replaces that variant's upgrade list; missing variants are unchanged. Key `variant_id`.

Failure/unknown handling: compact failure marks live upgrades unknown for the hour but preserves accumulated catalog.

R2 delta: preserve numeric `upgrade_index` names until the serial worker joins index-to-token metadata.

Contract wording: `39 air_variant_upgrades - PRODUCTION_SLICE_DONE; recorder_host decodes live air variant upgrade rows; folded into REPLAY_SCHEMA section 11 equipment_variants_live upgrade_levels with catalog-upsert semantics and failure-null behavior.`

Draft module: `fold_d39.rs`.

## D40 stockpile_variant_upgrades

Shape source: `recorder_host/src/main.rs::decode_compact_variant_upgrades_data_payload` for stockpile variants.

Decoded row shape: same wire shape as D39: header fields, variant rows with `upgrade_first`/`upgrade_count`, and upgrade rows with `upgrade_index`/`level`.

Target schema: section 11 live stockpile/design variants, best as `designs._r2_stockpile_variant_upgrades` unless the serial worker maps it into a distinct `equipment_variants_live` source namespace.

Fold semantics: accumulating catalog upsert. The row replaces that variant's upgrade list; missing variants are unchanged. Key `variant_id`.

Failure/unknown handling: compact failure marks stockpile upgrades unknown for the hour but preserves accumulated catalog.

R2 delta: keep stockpile upgrades in a distinct `_r2_stockpile_variant_upgrades` namespace/source badge to avoid collision with D39 air variant upgrades.

Contract wording: `40 stockpile_variant_upgrades - PRODUCTION_SLICE_DONE; recorder_host decodes live stockpile variant upgrade rows; folded into REPLAY_SCHEMA section 11 R2 stockpile variant upgrades with catalog-upsert semantics and failure-null behavior.`

Draft module: `fold_d40.rs`.

## D42 air_combat_losses

Shape source: `recorder_host/src/main.rs::decode_compact_air_combat_losses_live_data_payload`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_air_wing_rows: u32`, `observed_equipment_rows: u32`, `loss_rows: u32`, `reinforcement_rows: u32`, `deleted_rows: u32`, `scale_raw: u32`, `reserved: u32`.
- Row: `owner_tag_db_id: u32`, `wing_id: u32`, `generation: u64`, `flags: u32`, `mission_mask: u32`, `mission_region: u32`, `old_count: u32`, `new_count: u32`, `count_delta: i32`, `reserved: u32`, `old_equipment_amount_sum: i64`, `new_equipment_amount_sum: i64`, `equipment_amount_delta: i64`.

Target schema: no exact current schema home for live air-wing loss/reinforcement deltas. Use only the raw R2 sidecar `strategic_air._r2_air_combat_losses_live`; do not inject recorder-supplied rows into `events[]`, which is left for replay-layer derived events. Because failure frames retain the prior tracker baseline, the next recovered valid delta may aggregate changes across the failure gap and is not exact per-hour proof.

Fold semantics: per-hour replace of the current raw sidecar payload. The collector always marks this as delta data, but it is not a state-map delta and does not require a prior baseline. Valid empty payload means measured no deltas; a resolved zero-current-wing frame may carry tombstone/loss rows from the collector strength tracker.

Failure/unknown handling: compact failure makes the event/sidecar leaf null/unknown for that hour; do not emit an empty event list on failure.

R2 delta: all raw row fields should be preserved because there is no exact schema event contract yet. Re-key owner TAG.

Contract wording: `42 air_combat_losses - PRODUCTION_SLICE_DONE; recorder_host decodes live air wing loss/reinforcement deltas; folded as REPLAY_SCHEMA R2 air_combat_losses sidecar only with measured-empty versus failure-null behavior; post-failure-gap deltas may aggregate across the gap; replay-layer events remain derived downstream.`

Draft module: `fold_d42.rs`.

## D46 air_variant_archetypes

Shape source: `lanes/air_wings_variants_lifecycle_live/src/domain46_air_variant_archetypes_live.rs`.

Decoded row shape:
- Header: `elapsed_wall_ms: u64`, `row_count: u32`, `changed_rows: u32`, `status_flags: u32`, `observed_variant_rows: u32`, `classified_rows: u32`, `unresolved_rows: u32`, `static_catalog_hash: u32`, `fixed_scale_raw: u32`, `reserved0: u32`, `reserved1: u32`.
- Row: `variant_id: u32`, `flags: u32`, `equipment_token_raw: u32`, `equipment_type_token_raw: u32`, `interface_type_raw: u32`, `air_superiority_weight_raw: i32`, `source_mask: u32`, `reserved: u32`.
- Per-row strings after fixed rows: `archetype_token: compact string`, `base_airframe_token: compact string`, `role_token: compact string`.

Target schema: section 11 `designs.equipment_variants_live[variant_id].archetype`/`_r2_air_variant_archetypes`.

Fold semantics: lane currently always sets `COMPACT_STATUS_DELTA_DATA`, but first successful record contains the implicit baseline. Draft behavior: first successful record clears/replaces and records baseline; later records upsert/delete. Key `variant_id`. A resolved zero-current-variant frame is valid and carries tombstones; source failure makes archetypes unknown and retains the accumulator.

Failure/unknown handling: compact failure marks archetypes unknown for the hour but preserves accumulated catalog.

R2 delta: preserve `source_mask`, raw token ids, `static_catalog_hash`, and `air_superiority_weight_raw` scale metadata under `_r2_air_variant_archetypes`.

Contract wording: `46 air_variant_archetypes - PRODUCTION_SLICE_DONE; lane decodes live air variant archetype rows; folded into REPLAY_SCHEMA section 11 equipment_variants_live archetype/R2 metadata with implicit-first-baseline semantics and failure-null behavior.`

Draft module: `fold_d46.rs`.

## Standalone Test Results

Command shape used after `C:\tmp` proved unwritable in this sandbox:

```powershell
$testDir = Join-Path (Get-Location) 'extractor/native/fold_packets/chunk2/.test_tmp'
New-Item -ItemType Directory -Force -Path $testDir | Out-Null
$env:TEMP = $testDir
$env:TMP = $testDir
rustc --edition=2021 --test extractor/native/fold_packets/chunk2/fold_dNN.rs -o $testDir/fold_dNN-test.exe
& $testDir/fold_dNN-test.exe --quiet
```

Result lines:
- `fold_d27 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d28 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d29 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d30 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d31 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d32 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d33 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d34 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d35 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d36 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d37 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d38 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d39 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d40 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d42 ok` - `test result: ok. 3 passed; 0 failed`
- `fold_d46 ok` - `test result: ok. 3 passed; 0 failed`

The `.test_tmp` directory was removed after the run.

## Serial Worker Rechecks

- Verify whether domains 29, 30, 31, 32, 34, 36, and 38 should start setting `COMPACT_STATUS_DELTA_DATA` in collector code. The drafts match current collector behavior by treating post-baseline payloads as sparse even when the flag is absent.
- Verify D46 lane semantics before using the generic `delta_before_baseline` guard. It currently always marks delta and must be accepted as an implicit baseline on first success.
- Verify exact row flag constants for each moved domain against collector constants. Draft tests use the observed low-bit deletion pattern and the named decoder/lane flags.
- Add schema/provenance registry entries for domains not already listed in `schema.py`/provenance folded-domain validation before adding them to `FOLDED_DOMAIN_IDS`.
- Decide exact R2 key names for D31, D35, D36, D37, D38, D40, D42, and D46 before serial integration. The draft names are intentionally explicit and conservative.
- For country-owned rows, run final output through real `resolve_country_key`; unresolved numeric ids must stay numeric with provenance notes rather than being dropped.
