# Replay Native Compact Domain Registry

This is the orchestrator-owned source of truth for compact domain IDs. Lane
subagents must not choose IDs independently. If a lane needs a new domain, the
orchestrator assigns it here first.

For scope-sensitive status, read `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
alongside this registry. The registry status is a compact label; the master
contract lists what each validated slice actually covers and what remains open
for full replay fidelity.

Registration rule: lane implementation lives in lane-owned collector/decoder
modules. Shared `collector/src/lib.rs` and `recorder_host/src/main.rs` changes
should be registration hooks only unless the orchestrator explicitly creates or
migrates the module interface.

Status vocabulary:

- `validated slice` means the named scope passed the DLL/compact/Rust replay
  gate at hourly-or-better cadence. It does not mean the whole gameplay concept
  is captured.
- `concept complete` is reserved for full replay coverage of the concept: every
  replay-relevant extractable field is captured, or each remaining field has an
  exact blocker or conclusive rejection.
- Open walls stay in the status label. Do not compress them back to
  `production-complete`.

## Active ID Map

| ID | Domain | Status | Owner lane |
| --- | --- | --- | --- |
| 10 | `fuel_live` | existing production compact domain | country economy |
| 11 | `production_live` | existing production compact domain | country economy |
| 12 | `focus_live` | existing base compact domain | focus base |
| 13 | `tech_live` | existing base compact domain | research base |
| 14 | `politics_live` | existing base compact domain | politics base |
| 15 | `politics_residuals` | existing base compact domain | politics base |
| 16 | `decisions_live` | existing base compact domain | decisions base |
| 17 | `diplomacy_live` | existing base compact domain | diplomacy base |
| 18 | `country_identity` | active integration: per-frame `tag_db_id` -> TAG map for world_fold country re-keying; cosmetic_tag/color omitted | country identity |
| 20 | `divisions_live` | existing compact domain | divisions |
| 21 | `land_combats_live` | existing compact domain; folded into REPLAY_SCHEMA §7 `combats[]` snapshots | land combat |
| 22 | `map_control_live` | existing compact domain | map control |
| 23 | `map_detail_live` | existing compact domain; folded into REPLAY_SCHEMA section 4 state resources/buildings/province_buildings plus R2 `map.map_detail_live` mirror | map detail |
| 24 | `weather_live` | validated slice: weather visual drivers plus sandstorm flag; projected under `map._r2_weather_live` as an R2-documented additive key pending schema-owner acceptance; enum/particle parity and internals open | weather visual drivers |
| 26 | `division_equipment_live` | existing compact domain; folded into `divisions[*].equipment` only when owner/division id joins uniquely to D20, with unmatched/ambiguous rows dropped and counted in provenance | divisions |
| 27 | `army_orders_live` | existing compact domain; projected under `order_of_battle.battle_plan_orders` as an R2-documented recorder shape pending schema-owner acceptance | army orders |
| 28 | `oob_command_live` | existing compact domain; projected under section 12 `order_of_battle` command/fleet arrays with `_r2_oob_command_capture`, pending schema-owner acceptance of the live raw shape | army OOB |
| 29 | `ships_live` | existing compact domain; projected under `order_of_battle.ships` and task-force ship lists as an R2-documented recorder shape pending schema-owner acceptance | naval OOB |
| 30 | `air_wings_live` | existing compact domain; D30 owns section 12 `order_of_battle.air_wings` existence with `_r2_air_wing_capture` evidence | air wings |
| 31 | `air_region_activity_live` | existing compact domain; folded into REPLAY_SCHEMA sections 13.1/14 `_r2_air_region_activity` | air activity |
| 32 | `air_wing_loadouts_live` | existing compact domain; attaches section 12 air-wing loadout/equipment annotations only to D30-owned air-wing rows, with orphan rows dropped and noted as R2 provenance | air wings |
| 33 | `air_variant_defs` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live` definitions | air variants |
| 34 | `air_base_status_live` | existing compact domain; folded into REPLAY_SCHEMA section 14 `strategic_air.air_bases` | air bases |
| 35 | `air_variant_stats` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live.live_stats` | air variants |
| 36 | `air_variant_reserves` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs._r2_air_variant_reserves` | air reserves |
| 37 | `stockpile_variant_defs` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs._r2_stockpile_variant_defs` | stockpile |
| 38 | `air_superiority_inputs` | existing compact domain; folded into REPLAY_SCHEMA sections 13.1/14 `_r2_air_superiority_inputs` | air superiority |
| 39 | `air_variant_upgrades` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live.upgrade_levels` | air variants |
| 40 | `stockpile_variant_upgrades` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs._r2_stockpile_variant_upgrades` | stockpile |
| 41 | `projects_live` | partial validated slice: facilities/scientists/progress/completed bit folded into top-level `special_projects`; raw unresolved handles preserved under `_r2_*`; start/reward fixture blocked | projects |
| 42 | `air_combat_losses_live` | validated slice: loss/reinforcement delta; direct event root rejected; folded only into REPLAY_SCHEMA sections 13.1/14 `_r2_air_combat_losses_live`; not full air combat | air combat losses |
| 43 | `market_trade_live` | validated slice: purchase contracts plus MTD1 purchase-contract detail trailer (price/amount/speed/completed/days/delivery country/convoys/efficiency/progress); folded into REPLAY_SCHEMA section 7b `diplomacy_world.trade_contracts`, per-country `diplomacy.lend_lease`, and `r2_market_trade_live`; omitted candidates/full trade economy, MTLL, market stockpile, non-market contracts, and route/path semantics remain open | market/trade |
| 44 | `supply_logistics_attrition_live` | validated slice: supply/logistics subset folded into REPLAY_SCHEMA section 13.2 `supply.by_province` with `r2_logistics_attrition` badges; speed-five proof and exact province supply wall open | supply/logistics |
| 45 | `focus_research_decisions_depth_live` | validated slice: transition depth plus active targeted-timed country-target rows in D45 schema-v2 records selected by the H4CR record-header schema flag and sanity-checked with `FRD2`, folded as R2 focus/research/decision annotations and `countries[*].decisions._r2_depth_targets`; legacy v1 headers remain decoder-supported; D12 owns first-class focus fields, with D45 providing only an absent-only fallback for `focus.current`-style fields; numeric state/province targets, cooldowns, body fields, unmatched transitions, paused focus, and direct event hooks remain open/blocked | focus/research/decisions depth |
| 46 | `air_variant_archetypes_live` | validated slice: variant archetypes; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live` archetype/R2 metadata; token `20378` resolved; shared token hook/full air lifecycle open | air wings/variants lifecycle |
| 47 | `characters_leaders_cabinet_live` | shared integration pending: registered as a schedulable validated-slice/open-gap compact domain; folds validated live character identity, unit-leader identity/stats, country-leader role edges, and active ruler rows into `countries[*].commanders.leader` plus `_r2_characters_leaders_cabinet_live`; advisor roster, appointed advisor, character portrait, and unit-leader trait-token rows are blocked pending current-build `CAdvisor` / `CUnitLeaderTrait` revalidation; live speed-five proof and broader closure remain open | characters/leaders/cabinet |
| 48 | `construction_buildings_state_damage_live` | shared integration pending: validated offline compact slice for construction queue rows, all-or-none target state, numeric state/province building health rows, and numeric state resources; folded as R2/domain-owned sidecars to avoid D23 state-field ownership collisions; token/name, created date, exact target location, repair semantics/speed, and live proof remain fixture-blocked/open | construction/buildings/state damage |
| 49 | `naval_combat_lifecycle_live` | fixture-blocked: no active battle in current process; current CShip route validated | naval combat |
| 50 | `naval_combat_members_live` | fixture-blocked: no active battle in current process; current CShip route validated | naval combat |
| 51 | `scripted_state_flags_variables_live` | validated lane slice: shared registered and folded into `world`, country `extra`, and state `_r2_scripted_state` leaves; opaque-root statuses remain R2 provenance, not hard failures | scripted state/flags/variables |
| 52 | `occupation_resistance_compliance_live` | validated slice: shared registered occupation state rows folded into REPLAY_SCHEMA section 4 `states[*].occupation`; law/garrison extras retained under `_r2_occupation_resistance_compliance_live`; garrison scalar/collaboration detail open | occupation/resistance/compliance |
| 53 | `diplomacy_intel_operatives_live` | active integration, not production-scheduled: compact/offline contract exists for relation/opinion/subject rows, agency scalars plus probe-backed E4/F0 capacity slots, operative lifecycle rows, cryptology, compact state-event observations, and war tag-pair/casualty rows badged lifecycle-unvalidated, but current-build live root offsets are unvalidated and must not claim 870ebf source proof; operation detail, spy-network/intel-map percentages, incoming action queues, and full war/agency/operative semantics remain fixture-blocked/fail-closed | diplomacy/intel/operatives |
| 54 | `army_battleplans_depth_live` | shared integration pending: schedulable validated slice for active-army order rows, exact type-5 target states, live-`870ebf` save/live paired non-type-5 `order+0x70/+0x78` path geometry, and same-frame assignment/linkage status; direct roots, redraw/toggle lifecycle, activation/execution, stance/aggressiveness, non-path geometry, and full OOB reconstruction remain fixture-blocked/fail-closed | army OOB/orders/battleplans depth |
| 55 | `land_combat_detail_events_live` | shared integration pending: registered in collector/recorder and folded onto active combat snapshots; live proof + detailed combat fields open | land combat details |
| 56 | `naval_missions_status_live` | shared integration pending: registered validated task-force status slice folded as R2 `order_of_battle.extra`; full-replace rows for task_force_id/fleet_id/owner/current province, optional derived region, and aggregate sunk-convoy count; mission/home-base/repair/retreat/route fixtures open | naval missions/status |
| 57 | `session_multiplayer_metadata_live` | live player-country + pause roots validated (2026-07-02 passive probe); shared integration pending as a partial slice; game-speed root + MP-only lobby/player fields open | session/multiplayer metadata |
| 58 | `country_economy_counters_live` | assigned; no accepted lane output after usage-limit interruption; factory/resource/consumer-goods UI counter closure open | country economy counters |
| 59 | `templates_equipment_catalog_live` | strict vanilla+mod static catalog proof passed (2026-07-02); production replay packing shipped (`recorder_host --static-templates-equipment-catalog`: hashed `role="static_artifact"` entry + `index.json` `static_artifacts` roster, fail-closed absence); live template-id joins open | templates/equipment catalog |
| 1000 | `frame` | compact stream framing/perf records | recorder infrastructure |

## Lane Acceptance Under This Registry

- A lane may not report a new compact domain ID as free unless this registry
  assigns it.
- A lane-local proof must use its assigned final ID.
- A lane-local packet that still needs major shared collector/recorder
  implementation is not merge-ready.
- Final integration should be registration plus combined proof. If a lane cannot
  reach that shape, it must return an exact blocker or conclusive rejection.
- Recorder production-profile membership is derived from descriptor
  status/schedulability in `compact_contract`; do not maintain hand-copied
  profile lists in runners or docs.
